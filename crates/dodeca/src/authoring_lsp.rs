use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::{Facet, NumericType, PrimitiveType, Type, UserType};
use gingembre::ast::{Expr, Ident, Node, StringLit};
use gingembre::parser::Parser as TemplateParser;
use gingembre::semantic::{
    TemplateReferenceKind, TemplateSemanticIndex, TemplateSemanticTokenKind, TemplateSymbol,
    TemplateSymbolKind, TemplateSymbolOrigin,
};
use gingembre::{BUILTIN_FILTERS, BUILTIN_TESTS, BuiltinItemInfo, builtin_filter, builtin_test};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Command, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit, CreateFile,
    CreateFileOptions, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentChangeOperation, DocumentChanges, DocumentLink, DocumentLinkOptions,
    DocumentLinkParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    Documentation, ExecuteCommandOptions, ExecuteCommandParams, FileChangeType, FileSystemWatcher,
    GlobPattern, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, Location,
    MarkupContent, MarkupKind, MessageType, NumberOrString, OneOf,
    OptionalVersionedTextDocumentIdentifier, Position, PrepareRenameResponse, Range,
    ReferenceParams, Registration, RenameFile, RenameFileOptions, RenameOptions, RenameParams,
    ResourceOp, SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens,
    SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities, ServerInfo,
    ShowDocumentParams, SymbolInformation, SymbolKind, TextDocumentEdit,
    TextDocumentPositionParams, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
    WatchKind, WorkspaceEdit, WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::authoring_model::{
    AuthoringDocumentOverlay, AuthoringInputPath, AuthoringPage, AuthoringPageKind,
    AuthoringProject, AuthoringWorkspace, RenderedHref, RenderedHrefOrigin,
};
use crate::config::ResolvedConfig;
use crate::queries::{Frontmatter, default_title_from_source_path};
use crate::template_host::TEMPLATE_FUNCTION_NAMES;
use crate::types::SourcePath;

const LIST_PAGES_COMMAND: &str = "dodeca.listPages";
const DIAGNOSTICS_COMMAND: &str = "dodeca.authoringDiagnostics";
const CREATE_PAGE_COMMAND: &str = "dodeca.createPage";
const ROUTE_GRAPH_COMMAND: &str = "dodeca.routeGraph";

const TEMPLATE_SEMANTIC_TOKEN_VARIABLE: u32 = 0;
const TEMPLATE_SEMANTIC_TOKEN_PARAMETER: u32 = 1;
const TEMPLATE_SEMANTIC_TOKEN_PROPERTY: u32 = 2;
const TEMPLATE_SEMANTIC_TOKEN_FUNCTION: u32 = 3;
const TEMPLATE_SEMANTIC_TOKEN_MACRO: u32 = 4;
const TEMPLATE_SEMANTIC_TOKEN_STRING: u32 = 5;
const TEMPLATE_SEMANTIC_TOKEN_NUMBER: u32 = 6;
const TEMPLATE_SEMANTIC_TOKEN_KEYWORD: u32 = 7;

pub async fn run(content: Option<String>, output: Option<String>) -> Result<()> {
    let state = Arc::new(Mutex::new(AuthoringState {
        startup_args: LspStartupArgs { content, output },
        dirs: None,
        documents: HashMap::new(),
        input_revision: 0,
        workspace: None,
        applied_input_revision: None,
        world_cache: None,
    }));

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        state: Arc::clone(&state),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}

#[derive(Clone)]
struct Backend {
    client: Client,
    state: Arc<Mutex<AuthoringState>>,
}

struct AuthoringState {
    startup_args: LspStartupArgs,
    dirs: Option<AuthoringDirs>,
    documents: HashMap<Url, String>,
    input_revision: u64,
    workspace: Option<AuthoringWorkspace>,
    applied_input_revision: Option<u64>,
    world_cache: Option<CachedAuthoringWorld>,
}

#[derive(Debug, Clone)]
struct LspStartupArgs {
    content: Option<String>,
    output: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedAuthoringWorld {
    content_dir: Utf8PathBuf,
    input_revision: u64,
    world: AuthoringWorld,
}

#[derive(Debug, Clone)]
struct AuthoringWorld {
    project: AuthoringProject,
    template_index: TemplateAuthoringIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthoringDirs {
    content_dir: Utf8PathBuf,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> LspResult<InitializeResult> {
        let dirs = match self.resolve_dirs_from_initialize(&params) {
            Ok(dirs) => dirs,
            Err(err) => {
                self.client
                    .log_message(
                        MessageType::WARNING,
                        format!("deferred Dodeca project discovery: {err}"),
                    )
                    .await;
                None
            }
        };
        if let Some(dirs) = dirs {
            self.set_dirs(dirs);
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        LIST_PAGES_COMMAND.to_string(),
                        DIAGNOSTICS_COMMAND.to_string(),
                        CREATE_PAGE_COMMAND.to_string(),
                        ROUTE_GRAPH_COMMAND.to_string(),
                    ],
                    ..Default::default()
                }),
                definition_provider: Some(OneOf::Left(true)),
                references_provider: Some(OneOf::Left(true)),
                rename_provider: Some(OneOf::Right(RenameOptions {
                    prepare_provider: Some(true),
                    work_done_progress_options: Default::default(),
                })),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                document_link_provider: Some(DocumentLinkOptions {
                    resolve_provider: Some(false),
                    work_done_progress_options: Default::default(),
                }),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                semantic_tokens_provider: Some(
                    SemanticTokensServerCapabilities::SemanticTokensOptions(
                        SemanticTokensOptions {
                            legend: template_semantic_tokens_legend(),
                            full: Some(SemanticTokensFullOptions::Bool(true)),
                            range: Some(false),
                            work_done_progress_options: Default::default(),
                        },
                    ),
                ),
                completion_provider: Some(CompletionOptions {
                    resolve_provider: Some(false),
                    trigger_characters: Some(vec![
                        "(".to_string(),
                        "/".to_string(),
                        "@".to_string(),
                        "#".to_string(),
                        ".".to_string(),
                    ]),
                    ..CompletionOptions::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "dodeca-authoring".to_string(),
                version: Some(crate::dodeca_version().to_string()),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "dodeca authoring server initialized")
            .await;
        self.register_workspace_file_watches().await;
        self.publish_workspace_diagnostics().await;
    }

    async fn shutdown(&self) -> LspResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let content = params.text_document.text;
        self.set_document(uri.clone(), content.clone());
        self.publish_document_diagnostics(uri, content).await;
        self.publish_workspace_diagnostics().await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            let uri = params.text_document.uri;
            self.set_document(uri.clone(), change.text.clone());
            self.publish_document_diagnostics(uri, change.text).await;
            self.publish_workspace_diagnostics().await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let Some(text) = params.text else {
            return;
        };
        self.set_document(params.text_document.uri.clone(), text.clone());
        self.publish_document_diagnostics(params.text_document.uri, text)
            .await;
        self.publish_workspace_diagnostics().await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.remove_document(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
        self.publish_workspace_diagnostics().await;
    }

    async fn did_change_watched_files(&self, params: DidChangeWatchedFilesParams) {
        if let Err(err) = self.apply_watched_file_changes(params).await {
            self.client
                .log_message(MessageType::ERROR, err.to_string())
                .await;
        }
        self.publish_workspace_diagnostics().await;
    }

    // tower-lsp fixes execute-command responses to serde_json::Value at the protocol boundary.
    #[allow(clippy::disallowed_types)]
    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> LspResult<Option<serde_json::Value>> {
        match params.command.as_str() {
            LIST_PAGES_COMMAND => match self.list_pages().await {
                Ok(pages) => Ok(Some(pages_to_json(&pages))),
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, err.to_string())
                        .await;
                    Err(tower_lsp::jsonrpc::Error::internal_error())
                }
            },
            DIAGNOSTICS_COMMAND => match self.authoring_diagnostics().await {
                Ok(diagnostics) => Ok(Some(diagnostics_to_json(&diagnostics))),
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, err.to_string())
                        .await;
                    Err(tower_lsp::jsonrpc::Error::internal_error())
                }
            },
            ROUTE_GRAPH_COMMAND => match self.authoring_route_graph().await {
                Ok(graph) => Ok(Some(route_graph_to_json(&graph))),
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, err.to_string())
                        .await;
                    Err(tower_lsp::jsonrpc::Error::internal_error())
                }
            },
            CREATE_PAGE_COMMAND => match self.create_page_from_command(params.arguments).await {
                Ok(value) => Ok(Some(value)),
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, err.to_string())
                        .await;
                    Err(tower_lsp::jsonrpc::Error::internal_error())
                }
            },
            _ => Err(tower_lsp::jsonrpc::Error::invalid_request()),
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> LspResult<Option<CodeActionResponse>> {
        match self.code_actions(params).await {
            Ok(actions) => Ok(Some(actions)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn completion(&self, params: CompletionParams) -> LspResult<Option<CompletionResponse>> {
        match self.completions(params).await {
            Ok(items) => Ok(Some(CompletionResponse::Array(items))),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn hover(&self, params: HoverParams) -> LspResult<Option<Hover>> {
        match self
            .hover_for_position(
                &params.text_document_position_params.text_document.uri,
                params.text_document_position_params.position,
            )
            .await
        {
            Ok(hover) => Ok(hover),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn document_link(
        &self,
        params: DocumentLinkParams,
    ) -> LspResult<Option<Vec<DocumentLink>>> {
        match self.document_links(params).await {
            Ok(links) => Ok(Some(links)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> LspResult<Option<DocumentSymbolResponse>> {
        match self.document_symbols(params).await {
            Ok(symbols) => Ok(Some(DocumentSymbolResponse::Nested(symbols))),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> LspResult<Option<Vec<SymbolInformation>>> {
        match self.workspace_symbols(params).await {
            Ok(symbols) => Ok(Some(symbols)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> LspResult<Option<GotoDefinitionResponse>> {
        match self
            .definition_for_position(
                &params.text_document_position_params.text_document.uri,
                params.text_document_position_params.position,
            )
            .await
        {
            Ok(Some(location)) => Ok(Some(GotoDefinitionResponse::Scalar(location))),
            Ok(None) => Ok(None),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn references(&self, params: ReferenceParams) -> LspResult<Option<Vec<Location>>> {
        match self
            .references_for_position(
                &params.text_document_position.text_document.uri,
                params.text_document_position.position,
            )
            .await
        {
            Ok(locations) => Ok(Some(locations)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> LspResult<Option<PrepareRenameResponse>> {
        match self
            .prepare_rename_for_position(&params.text_document.uri, params.position)
            .await
        {
            Ok(response) => Ok(response),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn rename(&self, params: RenameParams) -> LspResult<Option<WorkspaceEdit>> {
        match self
            .rename_for_position(
                &params.text_document_position.text_document.uri,
                params.text_document_position.position,
                &params.new_name,
            )
            .await
        {
            Ok(edit) => Ok(edit),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> LspResult<Option<SemanticTokensResult>> {
        match self
            .semantic_tokens_for_document(&params.text_document.uri)
            .await
        {
            Ok(tokens) => Ok(tokens.map(SemanticTokensResult::Tokens)),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Ok(None)
            }
        }
    }
}

impl Backend {
    fn set_dirs(&self, dirs: AuthoringDirs) {
        let mut state = self.state.lock().unwrap();
        if state.dirs.as_ref() != Some(&dirs) {
            state.workspace = None;
            state.applied_input_revision = None;
            state.world_cache = None;
        }
        state.dirs = Some(dirs);
    }

    fn resolve_dirs_from_initialize(
        &self,
        params: &InitializeParams,
    ) -> Result<Option<AuthoringDirs>> {
        let startup_args = self.state.lock().unwrap().startup_args.clone();
        resolve_initial_authoring_dirs(&startup_args, params)
    }

    fn dirs(&self) -> Result<AuthoringDirs> {
        let state = self.state.lock().unwrap();
        state
            .dirs
            .clone()
            .ok_or_else(|| eyre!("dodeca authoring server has not been initialized"))
    }

    fn dirs_for_uri(&self, uri: &Url) -> Result<AuthoringDirs> {
        let startup_args = self.state.lock().unwrap().startup_args.clone();
        let dirs = resolve_authoring_dirs_for_document(&startup_args, uri)?;
        self.set_dirs(dirs.clone());
        Ok(dirs)
    }

    #[allow(clippy::disallowed_types)]
    async fn register_workspace_file_watches(&self) {
        let watchers = vec![FileSystemWatcher {
            glob_pattern: GlobPattern::String("**/*".to_string()),
            kind: Some(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
        }];
        let options = DidChangeWatchedFilesRegistrationOptions { watchers };
        let registration = Registration {
            id: "dodeca-authoring-inputs".to_string(),
            method: "workspace/didChangeWatchedFiles".to_string(),
            register_options: serde_json::to_value(options).ok(),
        };

        if let Err(err) = self.client.register_capability(vec![registration]).await {
            self.client
                .log_message(
                    MessageType::WARNING,
                    format!("could not register Dodeca file watches: {err}"),
                )
                .await;
        }
    }

    async fn apply_watched_file_changes(&self, params: DidChangeWatchedFilesParams) -> Result<()> {
        let dirs = self.dirs()?;
        let mut changed = false;
        {
            let mut state = self.state.lock().unwrap();
            let needs_workspace = state
                .workspace
                .as_ref()
                .is_none_or(|workspace| workspace.content_dir() != dirs.content_dir);

            if needs_workspace {
                state.workspace = Some(AuthoringWorkspace::new(&dirs.content_dir)?);
                state.applied_input_revision = None;
                state.world_cache = None;
            }

            let open_documents = state.documents.keys().cloned().collect::<HashSet<_>>();
            let workspace = state
                .workspace
                .as_mut()
                .expect("authoring workspace initialized");
            for event in params.changes {
                let path = lsp_file_uri_to_utf8_path(&event.uri)?;
                let Some(input_path) = workspace.input_path_for_absolute_path(&path)? else {
                    continue;
                };
                if open_documents.contains(&event.uri) {
                    continue;
                }
                let content = match (&input_path, event.typ == FileChangeType::DELETED) {
                    (_, true)
                    | (AuthoringInputPath::Static(_), false)
                    | (AuthoringInputPath::Dist(_), false) => None,
                    (_, false) => Some(std::fs::read_to_string(&path)?),
                };
                workspace.apply_file_change(&input_path, content.as_deref())?;
                changed = true;
            }

            if changed {
                state.input_revision = state.input_revision.wrapping_add(1);
                state.applied_input_revision = Some(state.input_revision);
                state.world_cache = None;
            }
        }

        Ok(())
    }

    async fn list_pages(&self) -> Result<Vec<AuthoringPage>> {
        let dirs = self.dirs()?;
        Ok(self.current_project(&dirs).await?.pages)
    }

    async fn authoring_diagnostics(&self) -> Result<Vec<AuthoringDiagnostic>> {
        let dirs = self.dirs()?;
        let project = self.current_project(&dirs).await?;
        Ok(load_authoring_diagnostics(&project))
    }

    async fn authoring_route_graph(&self) -> Result<Vec<RouteGraphNode>> {
        let dirs = self.dirs()?;
        let project = self.current_project(&dirs).await?;
        Ok(route_graph_for_project(&project))
    }

    fn set_document(&self, uri: Url, content: String) {
        let mut state = self.state.lock().unwrap();
        state.documents.insert(uri, content);
        state.input_revision = state.input_revision.wrapping_add(1);
        state.world_cache = None;
    }

    fn remove_document(&self, uri: &Url) {
        let mut state = self.state.lock().unwrap();
        state.documents.remove(uri);
        state.input_revision = state.input_revision.wrapping_add(1);
        state.world_cache = None;
    }

    fn document_content(&self, uri: &Url) -> Result<String> {
        if let Some(content) = self.state.lock().unwrap().documents.get(uri).cloned() {
            return Ok(content);
        }

        let path = uri
            .to_file_path()
            .map_err(|_| eyre!("LSP document URI is not a file URI: {uri}"))?;
        Ok(std::fs::read_to_string(path)?)
    }

    async fn current_project(&self, dirs: &AuthoringDirs) -> Result<AuthoringProject> {
        Ok(self.current_world(dirs).await?.project)
    }

    async fn current_world(&self, dirs: &AuthoringDirs) -> Result<AuthoringWorld> {
        let cached = {
            let state = self.state.lock().unwrap();
            state
                .world_cache
                .as_ref()
                .filter(|cached| {
                    cached.content_dir == dirs.content_dir
                        && cached.input_revision == state.input_revision
                })
                .map(|cached| cached.world.clone())
        };
        if let Some(world) = cached {
            return Ok(world);
        }

        let (inputs, inputs_revision) = {
            let mut state = self.state.lock().unwrap();
            let revision = state.input_revision;
            let needs_workspace = state
                .workspace
                .as_ref()
                .is_none_or(|workspace| workspace.content_dir() != dirs.content_dir);

            if needs_workspace {
                state.workspace = Some(AuthoringWorkspace::new(&dirs.content_dir)?);
                state.applied_input_revision = None;
                state.world_cache = None;
            }

            if state.applied_input_revision != Some(revision) {
                let documents = state.documents.clone();
                let workspace = state
                    .workspace
                    .as_mut()
                    .expect("authoring workspace initialized");
                let overlays = documents
                    .iter()
                    .filter_map(|(uri, content)| {
                        let path = lsp_file_uri_to_utf8_path(uri).ok()?;
                        let input_path = workspace.input_path_for_absolute_path(&path).ok()??;
                        Some(AuthoringDocumentOverlay {
                            path: input_path,
                            content: content.clone(),
                        })
                    })
                    .collect::<Vec<_>>();
                state
                    .workspace
                    .as_mut()
                    .expect("authoring workspace initialized")
                    .apply_overlays(&overlays)?;
                state.applied_input_revision = Some(revision);
            }

            (
                state
                    .workspace
                    .as_ref()
                    .expect("authoring workspace initialized")
                    .inputs(),
                revision,
            )
        };

        let project = inputs.project().await?;
        let world = AuthoringWorld {
            template_index: TemplateAuthoringIndex::new(&project),
            project,
        };

        let mut state = self.state.lock().unwrap();
        if state.input_revision == inputs_revision {
            state.world_cache = Some(CachedAuthoringWorld {
                content_dir: dirs.content_dir.clone(),
                input_revision: inputs_revision,
                world: world.clone(),
            });
        }

        Ok(world)
    }

    async fn code_actions(&self, params: CodeActionParams) -> Result<CodeActionResponse> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let project = self.current_project(&dirs).await?;
        let diagnostics = diagnostics_for_uri(&dirs.content_dir, &project, &uri, &content)?;
        let lsp_diagnostics = diagnostics
            .iter()
            .map(authoring_diagnostic_to_lsp)
            .collect::<Vec<_>>();

        let mut actions = Vec::new();
        if let Some(action) =
            extract_page_code_action(&dirs.content_dir, &project, &uri, &content, params.range)?
        {
            actions.push(action);
        }
        for diagnostic in diagnostics
            .into_iter()
            .filter(|diagnostic| ranges_overlap(&diagnostic.range(), &params.range))
        {
            match diagnostic.kind {
                AuthoringDiagnosticKind::Route => actions.extend(missing_route_code_actions(
                    &uri,
                    &diagnostic,
                    &lsp_diagnostics,
                )),
                AuthoringDiagnosticKind::Anchor => actions.extend(missing_anchor_code_actions(
                    &dirs.content_dir,
                    &project,
                    &content,
                    &diagnostic,
                    &lsp_diagnostics,
                )),
                AuthoringDiagnosticKind::Source
                | AuthoringDiagnosticKind::StaticAsset
                | AuthoringDiagnosticKind::Frontmatter
                | AuthoringDiagnosticKind::MissingTemplate
                | AuthoringDiagnosticKind::MissingBlock
                | AuthoringDiagnosticKind::UnknownMacro
                | AuthoringDiagnosticKind::UnknownFilter
                | AuthoringDiagnosticKind::UnknownTest
                | AuthoringDiagnosticKind::DuplicateTitle
                | AuthoringDiagnosticKind::DuplicateRoute
                | AuthoringDiagnosticKind::OrphanPage
                | AuthoringDiagnosticKind::NoInboundLinks => {}
            }
        }

        Ok(actions)
    }

    async fn completions(&self, params: CompletionParams) -> Result<Vec<CompletionItem>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let path = lsp_file_uri_to_utf8_path(&uri)?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let project = self.current_project(&dirs).await?;
            return Ok(template_completion_items(
                &project,
                &template_file,
                &content,
                position,
            ));
        }

        if let Some(context) = frontmatter_completion_context(&content, position) {
            let source_file = source_file_for_path(&dirs.content_dir, &path)?;
            return Ok(completion_items_for_frontmatter(&source_file, &context));
        }

        let Some(context) = markdown_target_context_at_position(&content, position) else {
            return Ok(Vec::new());
        };

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        Ok(completion_items_for_markdown_target(
            &project, page, &context,
        ))
    }

    async fn hover_for_position(&self, uri: &Url, position: Position) -> Result<Option<Hover>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let world = self.current_world(&dirs).await?;
        let project = &world.project;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let template_index = &world.template_index;
            if let Some(occurrence) =
                template_index.block_occurrence_at_position(&template_file, position)
            {
                return Ok(Some(markdown_hover(
                    template_block_hover_markdown(template_index, &template_file, &occurrence),
                    occurrence.source_range,
                )));
            }
            if let Some(reference) =
                template_route_reference_at_position(project, &template_file, &content, position)
                && let Some(target_page) = project.page_for_route(&reference.target_route)
            {
                let (_, fragment) = split_fragment(&reference.target);
                return Ok(Some(markdown_hover(
                    page_link_hover_markdown(project, target_page, fragment),
                    reference.source_range,
                )));
            }
            if let Some(target) =
                template_document_target_at_position(project, &template_file, &content, position)?
            {
                return Ok(Some(markdown_hover(
                    target.hover_markdown(),
                    target.source_range,
                )));
            }
            if let Some(target) =
                template_definition_target_at_position(project, &template_file, &content, position)?
            {
                return Ok(Some(markdown_hover(
                    target.hover_markdown(),
                    target.source_range,
                )));
            }
            if let Some(hover) =
                template_semantic_hover(project, &template_file, &content, position)
            {
                return Ok(Some(hover));
            }
            return Ok(None);
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(target) = frontmatter_document_target_at_position(project, &content, position)?
        {
            return Ok(Some(markdown_hover(
                target.hover_markdown(),
                target.source_range,
            )));
        }

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            let backlink_count = references_to_page(&dirs.content_dir, project, page)?.len();
            return Ok(Some(markdown_hover(
                frontmatter_hover_markdown(project, page, &content, backlink_count),
                frontmatter_range,
            )));
        }

        let Some(reference) = reference_at_position(&content, position) else {
            return Ok(None);
        };
        let range = byte_range_to_lsp_range(&content, reference.byte_start, reference.byte_end);
        Ok(Some(markdown_hover(
            link_hover_markdown(project, page, &content, &reference),
            range,
        )))
    }

    async fn document_links(&self, params: DocumentLinkParams) -> Result<Vec<DocumentLink>> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let project = self.current_project(&dirs).await?;
        let path = lsp_file_uri_to_utf8_path(&uri)?;

        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let mut links = template_document_targets(&project, &template_file, &content)?
                .into_iter()
                .map(|target| {
                    Ok(DocumentLink {
                        range: target.source_range,
                        target: Some(target.target_uri()?),
                        tooltip: Some(target.tooltip()),
                        data: None,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            links.extend(
                template_route_references(&project, &template_file, &content)
                    .into_iter()
                    .filter_map(|reference| {
                        let source_file = project.source_file_for_route(&reference.target_route)?;
                        Some(DocumentLink {
                            range: reference.source_range,
                            target: Some(
                                Url::from_file_path(dirs.content_dir.join(source_file)).ok()?,
                            ),
                            tooltip: Some(format!("Open Dodeca page `{}`", reference.target_route)),
                            data: None,
                        })
                    }),
            );
            return Ok(links);
        }

        frontmatter_document_targets(&project, &content)?
            .into_iter()
            .map(|target| {
                Ok(DocumentLink {
                    range: target.source_range,
                    target: Some(target.target_uri()?),
                    tooltip: Some(target.tooltip()),
                    data: None,
                })
            })
            .collect()
    }

    async fn semantic_tokens_for_document(&self, uri: &Url) -> Result<Option<SemanticTokens>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? else {
            return Ok(None);
        };
        let project = self.current_world(&dirs).await?.project;
        let Some(index) = project.template_semantics.get(&template_file) else {
            return Ok(None);
        };

        Ok(Some(SemanticTokens {
            result_id: None,
            data: template_semantic_tokens(index, &content),
        }))
    }

    async fn document_symbols(&self, params: DocumentSymbolParams) -> Result<Vec<DocumentSymbol>> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let path = lsp_file_uri_to_utf8_path(&uri)?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let project = self.current_project(&dirs).await?;
            return template_document_symbols(&project, &template_file, &content);
        }
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
        let Some(page) = project.page_for_source_file(&source_file) else {
            return Ok(Vec::new());
        };

        Ok(vec![document_symbol_for_page(page, &content)])
    }

    async fn workspace_symbols(
        &self,
        params: WorkspaceSymbolParams,
    ) -> Result<Vec<SymbolInformation>> {
        let dirs = self.dirs()?;
        let project = self.current_project(&dirs).await?;
        Ok(workspace_symbols_for_project(
            &dirs.content_dir,
            &project,
            &params.query,
        ))
    }

    async fn definition_for_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Option<Location>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let world = self.current_world(&dirs).await?;
        let project = &world.project;
        let path = lsp_file_uri_to_utf8_path(uri)?;

        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let template_index = &world.template_index;
            if let Some(occurrence) =
                template_index.block_occurrence_at_position(&template_file, position)
            {
                if let Some(target) =
                    template_index.block_definition_target(&template_file, &occurrence)
                {
                    return Ok(Some(target.location()?));
                }
                return Ok(Some(Location {
                    uri: Url::from_file_path(
                        project
                            .template_paths
                            .get(&template_file)
                            .ok_or_else(|| eyre!("missing template path for {template_file}"))?
                            .as_std_path(),
                    )
                    .map_err(|_| {
                        eyre!("could not convert template path to URI: {template_file}")
                    })?,
                    range: occurrence.source_range,
                }));
            }
            if let Some(reference) =
                template_route_reference_at_position(project, &template_file, &content, position)
            {
                let (_, fragment) = split_fragment(&reference.target);
                if let Some(source_file) = project.source_file_for_route(&reference.target_route) {
                    let path = dirs.content_dir.join(source_file);
                    return location_for_source_path(&path, fragment);
                }
            }
            if let Some(target) =
                template_document_target_at_position(project, &template_file, &content, position)?
            {
                return Ok(Some(Location {
                    uri: target.target_uri()?,
                    range: one_line_range(0),
                }));
            }
            if let Some(target) =
                template_definition_target_at_position(project, &template_file, &content, position)?
            {
                return Ok(Some(target.location()?));
            }
            if let Some(location) =
                template_semantic_definition(&dirs.content_dir, project, &template_file, position)
            {
                return Ok(Some(location));
            }
            return Ok(None);
        }

        if let Some(target) = frontmatter_document_target_at_position(project, &content, position)?
        {
            return Ok(Some(Location {
                uri: target.target_uri()?,
                range: one_line_range(0),
            }));
        }

        let Some(reference) = reference_at_position(&content, position) else {
            return Ok(None);
        };

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        definition_for_reference(&dirs, project, page, &reference)
    }

    async fn references_for_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Vec<Location>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let world = self.current_world(&dirs).await?;
        let project = &world.project;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let template_index = &world.template_index;
            if let Some(occurrence) =
                template_index.block_occurrence_at_position(&template_file, position)
            {
                return template_block_references(template_index, &template_file, &occurrence.name);
            }
            if let Some(target) =
                template_document_target_at_position(project, &template_file, &content, position)?
            {
                return template_document_references(
                    &dirs.content_dir,
                    project,
                    &target.target_path,
                );
            }
            if let Some(query) = template_index.macro_reference_query(&template_file, position) {
                return template_macro_references(
                    template_index,
                    &query.target_template_file,
                    &query.macro_name,
                );
            }
            if let Some(reference) =
                template_route_reference_at_position(project, &template_file, &content, position)
                && let Some(target_page) = project.page_for_route(&reference.target_route)
            {
                return references_to_page(&dirs.content_dir, project, target_page);
            }
            return Ok(template_semantic_references(
                &dirs.content_dir,
                project,
                &template_file,
                &content,
                position,
            ));
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(target) = frontmatter_document_target_at_position(project, &content, position)?
            && target.kind == FrontmatterDocumentKind::Template
        {
            return template_document_references(&dirs.content_dir, project, &target.target_path);
        }

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            return references_to_page(&dirs.content_dir, project, page);
        }

        if let Some(heading_id) = heading_id_at_position(page, &content, position) {
            return references_to_heading(&dirs.content_dir, project, page, &heading_id);
        }

        Ok(Vec::new())
    }

    async fn prepare_rename_for_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Option<PrepareRenameResponse>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let project = self.current_project(&dirs).await?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            return Ok(template_semantic_prepare_rename(
                &project,
                &template_file,
                &content,
                position,
            ));
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                range: frontmatter_range,
                placeholder: page.route.clone(),
            }));
        }

        let Some(target) = heading_rename_target_at_position(page, &content, position) else {
            return Ok(None);
        };

        Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
            range: target.title_range,
            placeholder: target.title,
        }))
    }

    async fn rename_for_position(
        &self,
        uri: &Url,
        position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let project = self.current_project(&dirs).await?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            return template_semantic_rename_workspace_edit(
                uri,
                &project,
                &template_file,
                &content,
                position,
                new_name,
            );
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            return rename_page_route_workspace_edit(&dirs.content_dir, &project, page, new_name);
        }

        let Some(target) = heading_rename_target_at_position(page, &content, position) else {
            return Ok(None);
        };

        rename_heading_workspace_edit(
            &dirs.content_dir,
            &project,
            page,
            &source_file,
            &content,
            &target,
            new_name,
        )
    }

    async fn publish_document_diagnostics(&self, uri: Url, content: String) {
        let dirs = match self.dirs_for_uri(&uri) {
            Ok(dirs) => dirs,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                self.client.publish_diagnostics(uri, Vec::new(), None).await;
                return;
            }
        };
        let project = match self.current_project(&dirs).await {
            Ok(project) => project,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                let diagnostics =
                    best_effort_frontmatter_diagnostics_for_uri(&dirs.content_dir, &uri, &content)
                        .unwrap_or_default()
                        .iter()
                        .map(authoring_diagnostic_to_lsp)
                        .collect::<Vec<_>>();
                self.client
                    .publish_diagnostics(uri, diagnostics, None)
                    .await;
                return;
            }
        };
        let path = match lsp_file_uri_to_utf8_path(&uri) {
            Ok(path) => path,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                self.client.publish_diagnostics(uri, Vec::new(), None).await;
                return;
            }
        };
        if let Some(template_file) =
            template_file_for_path(&dirs.content_dir, &path).unwrap_or(None)
        {
            let diagnostics = diagnostics_for_template(&project, &template_file, &content)
                .iter()
                .map(authoring_diagnostic_to_lsp)
                .collect::<Vec<_>>();
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
            return;
        }
        if !is_content_markdown_document(&dirs.content_dir, &uri) {
            self.client.publish_diagnostics(uri, Vec::new(), None).await;
            return;
        }
        let diagnostics = match diagnostics_for_uri(&dirs.content_dir, &project, &uri, &content) {
            Ok(diagnostics) => diagnostics
                .iter()
                .map(authoring_diagnostic_to_lsp)
                .collect::<Vec<_>>(),
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                Vec::new()
            }
        };

        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn publish_workspace_diagnostics(&self) {
        let dirs = match self.dirs() {
            Ok(dirs) => dirs,
            Err(_) => return,
        };
        let project = match self.current_project(&dirs).await {
            Ok(project) => project,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                return;
            }
        };
        let diagnostics = load_authoring_diagnostics(&project);
        let mut diagnostics_by_source: HashMap<String, Vec<Diagnostic>> = HashMap::new();
        for diagnostic in diagnostics {
            diagnostics_by_source
                .entry(diagnostic.source_file.clone())
                .or_default()
                .push(authoring_diagnostic_to_lsp(&diagnostic));
        }

        for page in &project.pages {
            let path = dirs.content_dir.join(&page.source_file);
            let Some(uri) = Url::from_file_path(path.as_std_path()).ok() else {
                continue;
            };
            let diagnostics = diagnostics_by_source
                .remove(&page.source_file)
                .unwrap_or_default();
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }

        for (template_file, path) in &project.template_paths {
            let Some(uri) = Url::from_file_path(path.as_std_path()).ok() else {
                continue;
            };
            let diagnostics = diagnostics_by_source
                .remove(template_file)
                .unwrap_or_default();
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
    }

    #[allow(clippy::disallowed_types)]
    async fn create_page_from_command(
        &self,
        arguments: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let (source_uri, route) = parse_create_page_command_arguments(&arguments)?;
        let dirs = self.dirs_for_uri(&source_uri)?;
        let source_file = source_file_for_new_route(&route)
            .ok_or_else(|| eyre!("cannot create page for route '{route}'"))?;
        let path = dirs.content_dir.join(&source_file);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let title = default_title_from_source_path(&source_file);
            std::fs::write(&path, page_frontmatter(&title))?;
        }

        let uri = Url::from_file_path(path.as_std_path())
            .map_err(|_| eyre!("created page path is not a file URI: {path}"))?;
        let _ = self
            .client
            .show_document(ShowDocumentParams {
                uri: uri.clone(),
                external: Some(false),
                take_focus: Some(true),
                selection: Some(Range {
                    start: Position {
                        line: 0,
                        character: 0,
                    },
                    end: Position {
                        line: 0,
                        character: 0,
                    },
                }),
            })
            .await;
        self.publish_workspace_diagnostics().await;

        Ok(created_page_to_json(&source_file, &route, &uri))
    }
}

fn resolve_initial_authoring_dirs(
    startup_args: &LspStartupArgs,
    params: &InitializeParams,
) -> Result<Option<AuthoringDirs>> {
    let complete_dir_override = startup_args.content.is_some() && startup_args.output.is_some();
    if complete_dir_override {
        let content_dir = Utf8PathBuf::from(
            startup_args
                .content
                .clone()
                .expect("complete override has content"),
        );
        return Ok(Some(authoring_dirs_from_config(content_dir)));
    }

    let Some(path) = project_path_from_initialize(params)? else {
        return Ok(None);
    };

    let cfg = ResolvedConfig::discover_containing(Utf8Path::new(&path))?;
    Ok(cfg.map(|cfg| authoring_dirs_from_resolved_config(startup_args, cfg)))
}

fn resolve_authoring_dirs_for_document(
    startup_args: &LspStartupArgs,
    uri: &Url,
) -> Result<AuthoringDirs> {
    let complete_dir_override = startup_args.content.is_some() && startup_args.output.is_some();
    if complete_dir_override {
        let content_dir = Utf8PathBuf::from(
            startup_args
                .content
                .clone()
                .expect("complete override has content"),
        );
        return Ok(authoring_dirs_from_config(content_dir));
    }

    let path = lsp_file_uri_to_utf8_path(uri)?;
    let cfg = ResolvedConfig::discover_containing(&path)?
        .ok_or_else(|| eyre!("No Dodeca configuration found for document {}", path))?;
    Ok(authoring_dirs_from_resolved_config(startup_args, cfg))
}

fn authoring_dirs_from_resolved_config(
    startup_args: &LspStartupArgs,
    cfg: ResolvedConfig,
) -> AuthoringDirs {
    let content_dir = startup_args
        .content
        .clone()
        .map(Utf8PathBuf::from)
        .unwrap_or(cfg.content_dir);
    authoring_dirs_from_config(content_dir)
}

fn authoring_dirs_from_config(content_dir: Utf8PathBuf) -> AuthoringDirs {
    AuthoringDirs { content_dir }
}

#[allow(deprecated)]
fn project_path_from_initialize(params: &InitializeParams) -> Result<Option<String>> {
    if let Some(folder) = params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
    {
        return lsp_file_uri_to_utf8_path(&folder.uri).map(|path| Some(path.to_string()));
    }

    if let Some(uri) = &params.root_uri {
        return lsp_file_uri_to_utf8_path(uri).map(|path| Some(path.to_string()));
    }

    Ok(params.root_path.clone())
}

fn lsp_file_uri_to_utf8_path(uri: &Url) -> Result<Utf8PathBuf> {
    let path = uri
        .to_file_path()
        .map_err(|_| eyre!("LSP workspace URI is not a file URI: {uri}"))?;
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| eyre!("LSP workspace path is not UTF-8: {}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthoringDiagnostic {
    source_file: String,
    route: String,
    kind: AuthoringDiagnosticKind,
    target: String,
    resolved_route: Option<String>,
    message: String,
    line: u32,
    column: u32,
    line_end: u32,
    column_end: u32,
    byte_start: usize,
    byte_end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteGraphNode {
    route: String,
    source_file: String,
    title: String,
    incoming: Vec<RouteGraphEdge>,
    outgoing: Vec<RouteGraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteGraphEdge {
    kind: RouteGraphEdgeKind,
    source_route: String,
    source_file: String,
    target_route: String,
    target: String,
    line: Option<u32>,
    column: Option<u32>,
    line_end: Option<u32>,
    column_end: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteGraphEdgeKind {
    Markdown,
    RenderedHtml,
}

impl RouteGraphEdgeKind {
    fn label(self) -> &'static str {
        match self {
            RouteGraphEdgeKind::Markdown => "markdown",
            RouteGraphEdgeKind::RenderedHtml => "renderedHtml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateRouteReference {
    target: String,
    target_route: String,
    source_range: Range,
}

#[derive(Debug, Clone)]
struct TemplateBlockOccurrence {
    name: String,
    source_range: Range,
}

#[derive(Debug, Clone)]
struct TemplateMacroOccurrence {
    name: String,
    source_range: Range,
}

#[derive(Debug, Clone)]
struct TemplateMacroCallOccurrence {
    target_template_file: String,
    macro_name: String,
    source_range: Range,
}

#[derive(Debug, Clone)]
struct TemplateAuthoringIndex {
    templates: HashMap<String, IndexedTemplate>,
    children_by_parent: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct IndexedTemplate {
    path: Utf8PathBuf,
    extends: Option<String>,
    blocks: Vec<TemplateBlockOccurrence>,
    macros: Vec<TemplateMacroOccurrence>,
    macro_calls: Vec<TemplateMacroCallOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateBlockReferenceTarget {
    path: Utf8PathBuf,
    range: Range,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateMacroReferenceQuery {
    target_template_file: String,
    macro_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TemplateMacroReferenceTarget {
    path: Utf8PathBuf,
    range: Range,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringDiagnosticKind {
    Route,
    Anchor,
    Source,
    StaticAsset,
    Frontmatter,
    MissingTemplate,
    MissingBlock,
    UnknownMacro,
    UnknownFilter,
    UnknownTest,
    DuplicateTitle,
    DuplicateRoute,
    OrphanPage,
    NoInboundLinks,
}

impl AuthoringDiagnostic {
    fn range(&self) -> Range {
        Range {
            start: Position {
                line: self.line.saturating_sub(1),
                character: self.column.saturating_sub(1),
            },
            end: Position {
                line: self.line_end.saturating_sub(1),
                character: self.column_end.saturating_sub(1),
            },
        }
    }
}

fn load_authoring_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        diagnostics.extend(diagnostics_for_page(project, page, content));
    }

    for (template_file, content) in &project.template_contents {
        diagnostics.extend(diagnostics_for_template(project, template_file, content));
    }
    diagnostics.extend(site_graph_diagnostics(project));

    diagnostics.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then_with(|| a.byte_start.cmp(&b.byte_start))
            .then_with(|| a.target.cmp(&b.target))
    });
    diagnostics
}

fn diagnostics_for_uri(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    uri: &Url,
    content: &str,
) -> Result<Vec<AuthoringDiagnostic>> {
    let path = Utf8PathBuf::from_path_buf(
        uri.to_file_path()
            .map_err(|_| eyre!("LSP document URI is not a file URI: {uri}"))?,
    )
    .map_err(|path| eyre!("LSP document path is not UTF-8: {}", path.display()))?;

    let source_file = source_file_for_path(content_dir, &path)?;
    let page = project
        .page_for_source_file(&source_file)
        .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

    Ok(diagnostics_for_page(project, page, content))
}

fn diagnostics_for_page(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics =
        frontmatter_diagnostics_for_source(&page.source_file, &page.route, content);
    diagnostics.extend(
        markdown_references(content)
            .into_iter()
            .filter_map(|reference| diagnostic_for_reference(project, page, content, reference)),
    );
    diagnostics
}

fn diagnostics_for_template(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Vec<AuthoringDiagnostic> {
    let Ok(template) = TemplateParser::new(template_file, content).parse() else {
        return Vec::new();
    };

    let mut diagnostics = Vec::new();
    let imports = template_import_aliases(project, &template.body);
    let parent_file = template_extends_path(template_file, content, &mut HashSet::new())
        .filter(|path| project.template_paths.contains_key(path));
    collect_template_diagnostics(
        project,
        template_file,
        content,
        &template.body,
        parent_file.as_deref(),
        &imports,
        &mut diagnostics,
    );
    diagnostics
}

fn site_graph_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.extend(duplicate_title_diagnostics(project));
    diagnostics.extend(duplicate_route_diagnostics(project));
    diagnostics.extend(inbound_link_diagnostics(project));
    diagnostics
}

fn route_graph_for_project(project: &AuthoringProject) -> Vec<RouteGraphNode> {
    let mut outgoing_by_route: HashMap<String, Vec<RouteGraphEdge>> = HashMap::new();
    let mut incoming_by_route: HashMap<String, Vec<RouteGraphEdge>> = HashMap::new();
    let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        for reference in markdown_references(content) {
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if !project.route_exists(&target_route) {
                continue;
            }
            let (line, column) = byte_to_line_column(content, reference.byte_start);
            let (line_end, column_end) = byte_to_line_column(content, reference.byte_end);
            let edge = RouteGraphEdge {
                kind: RouteGraphEdgeKind::Markdown,
                source_route: page.route.clone(),
                source_file: page.source_file.clone(),
                target_route: target_route.clone(),
                target: reference.target,
                line: Some(line),
                column: Some(column),
                line_end: Some(line_end),
                column_end: Some(column_end),
            };
            seen_edges.insert((
                edge.source_route.clone(),
                edge.target_route.clone(),
                edge.target.clone(),
            ));
            outgoing_by_route
                .entry(page.route.clone())
                .or_default()
                .push(edge.clone());
            incoming_by_route
                .entry(target_route)
                .or_default()
                .push(edge);
        }
    }

    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if !seen_edges.insert((
                source_route.clone(),
                target_route.clone(),
                href.href.clone(),
            )) {
                continue;
            }
            let edge = RouteGraphEdge {
                kind: RouteGraphEdgeKind::RenderedHtml,
                source_route: source_route.clone(),
                source_file: source_page.source_file.clone(),
                target_route: target_route.clone(),
                target: href.href.clone(),
                line: None,
                column: None,
                line_end: None,
                column_end: None,
            };
            outgoing_by_route
                .entry(source_route.clone())
                .or_default()
                .push(edge.clone());
            incoming_by_route
                .entry(target_route)
                .or_default()
                .push(edge);
        }
    }

    project
        .pages
        .iter()
        .map(|page| RouteGraphNode {
            route: page.route.clone(),
            source_file: page.source_file.clone(),
            title: page.title.clone(),
            incoming: incoming_by_route.remove(&page.route).unwrap_or_default(),
            outgoing: outgoing_by_route.remove(&page.route).unwrap_or_default(),
        })
        .collect()
}

fn duplicate_title_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let mut by_title: HashMap<&str, Vec<&AuthoringPage>> = HashMap::new();
    for page in &project.pages {
        if !page.title.trim().is_empty() {
            by_title.entry(page.title.as_str()).or_default().push(page);
        }
    }

    by_title
        .into_iter()
        .filter(|(_, pages)| pages.len() > 1)
        .flat_map(|(title, pages)| {
            pages.into_iter().filter_map(move |page| {
                let content = project.source_contents.get(&page.source_file)?;
                Some(site_graph_diagnostic_for_page(
                    page,
                    content,
                    AuthoringDiagnosticKind::DuplicateTitle,
                    title,
                    format!("title '{title}' is used by multiple pages"),
                ))
            })
        })
        .collect()
}

fn duplicate_route_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let mut by_route: HashMap<&str, Vec<&AuthoringPage>> = HashMap::new();
    for page in &project.pages {
        by_route.entry(page.route.as_str()).or_default().push(page);
    }

    by_route
        .into_iter()
        .filter(|(_, pages)| pages.len() > 1)
        .flat_map(|(route, pages)| {
            pages.into_iter().filter_map(move |page| {
                let content = project.source_contents.get(&page.source_file)?;
                Some(site_graph_diagnostic_for_page(
                    page,
                    content,
                    AuthoringDiagnosticKind::DuplicateRoute,
                    route,
                    format!("route '{route}' is produced by multiple source files"),
                ))
            })
        })
        .collect()
}

fn inbound_link_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let inbound_counts = inbound_link_counts(project);
    project
        .pages
        .iter()
        .filter(|page| page.route != "/")
        .filter(|page| inbound_counts.get(&page.route).copied().unwrap_or(0) == 0)
        .filter(|page| !is_section_landing_with_children(project, page))
        .filter_map(|page| {
            let content = project.source_contents.get(&page.source_file)?;
            let kind = match page.kind {
                AuthoringPageKind::Page => AuthoringDiagnosticKind::OrphanPage,
                AuthoringPageKind::Section => AuthoringDiagnosticKind::NoInboundLinks,
            };
            Some(site_graph_diagnostic_for_page(
                page,
                content,
                kind,
                &page.route,
                format!("{} has no inbound links", page.route),
            ))
        })
        .collect()
}

fn inbound_link_counts(project: &AuthoringProject) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        for reference in markdown_references(content) {
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if project.route_exists(&target_route) && target_route != page.route {
                *counts.entry(target_route).or_insert(0) += 1;
            }
        }
    }
    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if project.route_exists(&target_route) && target_route != source_page.route {
                *counts.entry(target_route).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn rendered_href_target_route(
    project: &AuthoringProject,
    source_page: &AuthoringPage,
    href: &str,
) -> Option<String> {
    if is_special_target(href) {
        return None;
    }

    let (target_without_fragment, _) = split_fragment(href);
    if target_without_fragment.is_empty() || is_likely_static_file(target_without_fragment) {
        return None;
    }

    let target_route = route_for_link_target(project, source_page, target_without_fragment);
    project.route_exists(&target_route).then_some(target_route)
}

fn template_route_reference_at_position(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Option<TemplateRouteReference> {
    template_route_references(project, template_file, content)
        .into_iter()
        .find(|reference| range_contains_position(&reference.source_range, position))
}

fn template_route_references(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Vec<TemplateRouteReference> {
    let mut references = Vec::new();
    let mut seen = HashSet::new();

    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(origin) = &href.origin else {
                continue;
            };
            let AuthoringInputPath::Template(origin_template_file) = &origin.path else {
                continue;
            };
            if origin_template_file != template_file || origin.byte_end > content.len() {
                continue;
            }
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if !seen.insert((
                origin.byte_start,
                origin.byte_end,
                href.href.clone(),
                target_route.clone(),
            )) {
                continue;
            }
            references.push(TemplateRouteReference {
                target: href.href.clone(),
                target_route,
                source_range: byte_range_to_lsp_range(content, origin.byte_start, origin.byte_end),
            });
        }
    }

    references
}

fn is_section_landing_with_children(project: &AuthoringProject, page: &AuthoringPage) -> bool {
    if page.kind != AuthoringPageKind::Section {
        return false;
    }
    let prefix = if page.route == "/" {
        "/".to_string()
    } else {
        format!("{}/", page.route.trim_end_matches('/'))
    };
    project
        .pages
        .iter()
        .any(|candidate| candidate.route != page.route && candidate.route.starts_with(&prefix))
}

fn site_graph_diagnostic_for_page(
    page: &AuthoringPage,
    content: &str,
    kind: AuthoringDiagnosticKind,
    target: &str,
    message: String,
) -> AuthoringDiagnostic {
    let (byte_start, byte_end) = site_graph_page_identity_byte_range(content);
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    AuthoringDiagnostic {
        source_file: page.source_file.clone(),
        route: page.route.clone(),
        kind,
        target: target.to_string(),
        resolved_route: Some(page.route.clone()),
        message,
        line,
        column,
        line_end,
        column_end,
        byte_start,
        byte_end,
    }
}

fn site_graph_page_identity_byte_range(content: &str) -> (usize, usize) {
    if let Some(frontmatter_tail) = content.strip_prefix("+++\n") {
        let end = frontmatter_tail
            .find("\n+++")
            .map(|offset| 4 + offset + "\n+++".len())
            .unwrap_or(content.len());
        return (0, end);
    }
    let first_line_end = content.find('\n').unwrap_or(content.len());
    (0, first_line_end)
}

fn collect_template_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    nodes: &[Node],
    parent_file: Option<&str>,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    for node in nodes {
        match node {
            Node::Extends(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Extends,
                &node.path,
                diagnostics,
            ),
            Node::Include(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Include,
                &node.path,
                diagnostics,
            ),
            Node::Import(node) => push_missing_template_diagnostic(
                project,
                template_file,
                content,
                TemplateDocumentKind::Import,
                &node.path,
                diagnostics,
            ),
            Node::Block(node) => {
                if let Some(parent_file) = parent_file
                    && template_block_definition(
                        project,
                        parent_file,
                        template_file,
                        content,
                        &node.name.name,
                    )
                    .is_none()
                {
                    diagnostics.push(template_diagnostic_for_ident(
                        template_file,
                        content,
                        AuthoringDiagnosticKind::MissingBlock,
                        &node.name,
                        format!("parent block '{}' not found", node.name.name),
                    ));
                }
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
            }
            Node::Macro(node) => {
                for param in &node.params {
                    if let Some(default) = &param.default {
                        collect_template_expr_diagnostics(
                            project,
                            template_file,
                            content,
                            default,
                            imports,
                            diagnostics,
                        );
                    }
                }
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
            }
            Node::Print(node) => collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &node.expr,
                imports,
                diagnostics,
            ),
            Node::If(node) => {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.condition,
                    imports,
                    diagnostics,
                );
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.then_body,
                    parent_file,
                    imports,
                    diagnostics,
                );
                for branch in &node.elif_branches {
                    collect_template_expr_diagnostics(
                        project,
                        template_file,
                        content,
                        &branch.condition,
                        imports,
                        diagnostics,
                    );
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        &branch.body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
                if let Some(body) = &node.else_body {
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::For(node) => {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.iter,
                    imports,
                    diagnostics,
                );
                collect_template_diagnostics(
                    project,
                    template_file,
                    content,
                    &node.body,
                    parent_file,
                    imports,
                    diagnostics,
                );
                if let Some(body) = &node.else_body {
                    collect_template_diagnostics(
                        project,
                        template_file,
                        content,
                        body,
                        parent_file,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::Set(node) => collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &node.value,
                imports,
                diagnostics,
            ),
            Node::CallBlock(node) => {
                for (_, expr) in &node.kwargs {
                    collect_template_expr_diagnostics(
                        project,
                        template_file,
                        content,
                        expr,
                        imports,
                        diagnostics,
                    );
                }
            }
            Node::Text(_) | Node::Comment(_) | Node::Continue(_) | Node::Break(_) => {}
        }
    }
}

fn push_missing_template_diagnostic(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    kind: TemplateDocumentKind,
    path: &StringLit,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    if project.template_paths.contains_key(&path.value) {
        return;
    }
    diagnostics.push(template_diagnostic_for_span(
        template_file,
        content,
        AuthoringDiagnosticKind::MissingTemplate,
        &path.value,
        format!("template {} '{}' not found", kind.label(), path.value),
        path.span.offset(),
        path.span.offset() + path.span.len(),
    ));
}

fn collect_template_expr_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    expr: &Expr,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    match expr {
        Expr::Literal(literal) => collect_template_literal_diagnostics(
            project,
            template_file,
            content,
            literal,
            imports,
            diagnostics,
        ),
        Expr::Var(_) => {}
        Expr::Field(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                diagnostics,
            );
        }
        Expr::Index(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.index,
                imports,
                diagnostics,
            );
        }
        Expr::Filter(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
            if builtin_filter(&expr.filter.name).is_none() {
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownFilter,
                    &expr.filter,
                    format!("filter '{}' not found", expr.filter.name),
                ));
            }
        }
        Expr::Binary(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.left,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.right,
                imports,
                diagnostics,
            );
        }
        Expr::Unary(expr) => collect_template_expr_diagnostics(
            project,
            template_file,
            content,
            &expr.expr,
            imports,
            diagnostics,
        ),
        Expr::Call(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.func,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
        }
        Expr::Ternary(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.value,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.condition,
                imports,
                diagnostics,
            );
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.otherwise,
                imports,
                diagnostics,
            );
        }
        Expr::Test(expr) => {
            collect_template_expr_diagnostics(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                diagnostics,
            );
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            if builtin_test(&expr.test_name.name).is_none() {
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownTest,
                    &expr.test_name,
                    format!("test '{}' not found", expr.test_name.name),
                ));
            }
        }
        Expr::MacroCall(expr) => {
            for arg in &expr.args {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    diagnostics,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
            if template_macro_definition_target(
                project,
                template_file,
                content,
                imports,
                &expr.namespace,
                &expr.macro_name,
            )
            .is_none()
            {
                let target = format!("{}::{}", expr.namespace.name, expr.macro_name.name);
                diagnostics.push(template_diagnostic_for_ident(
                    template_file,
                    content,
                    AuthoringDiagnosticKind::UnknownMacro,
                    &expr.macro_name,
                    format!("macro '{target}' not found"),
                ));
            }
        }
    }
}

fn collect_template_literal_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    literal: &gingembre::ast::Literal,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    match literal {
        gingembre::ast::Literal::List(list) => {
            for expr in &list.elements {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    diagnostics,
                );
            }
        }
        gingembre::ast::Literal::Dict(dict) => {
            for (key, value) in &dict.entries {
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    key,
                    imports,
                    diagnostics,
                );
                collect_template_expr_diagnostics(
                    project,
                    template_file,
                    content,
                    value,
                    imports,
                    diagnostics,
                );
            }
        }
        gingembre::ast::Literal::String(_)
        | gingembre::ast::Literal::Int(_)
        | gingembre::ast::Literal::Float(_)
        | gingembre::ast::Literal::Bool(_)
        | gingembre::ast::Literal::None(_) => {}
    }
}

fn template_diagnostic_for_ident(
    template_file: &str,
    content: &str,
    kind: AuthoringDiagnosticKind,
    ident: &Ident,
    message: String,
) -> AuthoringDiagnostic {
    template_diagnostic_for_span(
        template_file,
        content,
        kind,
        &ident.name,
        message,
        ident.span.offset(),
        ident.span.offset() + ident.span.len(),
    )
}

fn template_diagnostic_for_span(
    template_file: &str,
    content: &str,
    kind: AuthoringDiagnosticKind,
    target: &str,
    message: String,
    byte_start: usize,
    byte_end: usize,
) -> AuthoringDiagnostic {
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    AuthoringDiagnostic {
        source_file: template_file.to_string(),
        route: String::new(),
        kind,
        target: target.to_string(),
        resolved_route: None,
        message,
        line,
        column,
        line_end,
        column_end,
        byte_start,
        byte_end,
    }
}

fn best_effort_frontmatter_diagnostics_for_uri(
    content_dir: &Utf8Path,
    uri: &Url,
    content: &str,
) -> Result<Vec<AuthoringDiagnostic>> {
    let path = lsp_file_uri_to_utf8_path(uri)?;
    let source_file = source_file_for_path(content_dir, &path)?;
    let route = SourcePath::new(source_file.clone()).to_route().to_string();
    Ok(frontmatter_diagnostics_for_source(
        &source_file,
        &route,
        content,
    ))
}

fn frontmatter_diagnostics_for_source(
    source_file: &str,
    route: &str,
    content: &str,
) -> Vec<AuthoringDiagnostic> {
    let known_fields = frontmatter_field_specs()
        .into_iter()
        .map(|spec| (spec.name, spec))
        .collect::<HashMap<_, _>>();
    let mut seen = HashSet::new();
    let mut diagnostics = Vec::new();

    for entry in frontmatter_entries(content) {
        if entry.table.as_deref().is_some_and(|table| table != "extra") {
            continue;
        }
        if entry.table.is_some() {
            continue;
        }

        let Some(spec) = known_fields.get(entry.key.as_str()) else {
            diagnostics.push(frontmatter_diagnostic(
                source_file,
                route,
                content,
                &entry.key,
                format!("unknown Dodeca frontmatter field '{}'", entry.key),
                entry.key_start,
                entry.key_end,
            ));
            continue;
        };

        if !seen.insert(entry.key.clone()) {
            diagnostics.push(frontmatter_diagnostic(
                source_file,
                route,
                content,
                &entry.key,
                format!("duplicate Dodeca frontmatter field '{}'", entry.key),
                entry.key_start,
                entry.key_end,
            ));
        }

        if spec.kind == FrontmatterFieldKind::Table {
            diagnostics.push(frontmatter_diagnostic(
                source_file,
                route,
                content,
                &entry.key,
                "frontmatter custom fields belong under an [extra] table".to_string(),
                entry.key_start,
                entry.key_end,
            ));
        } else if !frontmatter_value_matches_kind(&entry.value, spec.kind) {
            diagnostics.push(frontmatter_diagnostic(
                source_file,
                route,
                content,
                &entry.key,
                format!(
                    "frontmatter field '{}' expects {}",
                    entry.key,
                    spec.kind.description()
                ),
                entry.value_start,
                entry.value_end,
            ));
        }
    }

    diagnostics
}

fn frontmatter_diagnostic(
    source_file: &str,
    route: &str,
    content: &str,
    target: &str,
    message: String,
    byte_start: usize,
    byte_end: usize,
) -> AuthoringDiagnostic {
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    AuthoringDiagnostic {
        source_file: source_file.to_string(),
        route: route.to_string(),
        kind: AuthoringDiagnosticKind::Frontmatter,
        target: target.to_string(),
        resolved_route: None,
        message,
        line,
        column,
        line_end,
        column_end,
        byte_start,
        byte_end,
    }
}

fn diagnostic_for_reference(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
    reference: MarkdownReference,
) -> Option<AuthoringDiagnostic> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return None;
    }

    let (target_without_fragment, fragment) = split_fragment(target);
    let (kind, resolved_route, message) =
        if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
            let Some(route) = project.source_to_route.get(source_target) else {
                return Some(reference.diagnostic(
                    page,
                    content,
                    AuthoringDiagnosticKind::Source,
                    None,
                    format!("source file '{source_target}' not found"),
                ));
            };
            match missing_anchor_message(project, route, fragment) {
                Some(message) => (
                    AuthoringDiagnosticKind::Anchor,
                    Some(route.clone()),
                    message,
                ),
                None => return None,
            }
        } else if reference.kind == MarkdownReferenceKind::Image
            || is_likely_static_file(target_without_fragment)
        {
            if project.static_target_exists(&page.source_file, target_without_fragment) {
                return None;
            }
            (
                AuthoringDiagnosticKind::StaticAsset,
                None,
                format!("static asset '{target_without_fragment}' not found"),
            )
        } else {
            let target_route = route_for_link_target(project, page, target_without_fragment);

            if !project.route_exists(&target_route) {
                (
                    AuthoringDiagnosticKind::Route,
                    Some(target_route.clone()),
                    format!("route '{target_route}' not found"),
                )
            } else if let Some(message) = missing_anchor_message(project, &target_route, fragment) {
                (AuthoringDiagnosticKind::Anchor, Some(target_route), message)
            } else {
                return None;
            }
        };

    Some(reference.diagnostic(page, content, kind, resolved_route, message))
}

fn missing_route_code_actions(
    uri: &Url,
    diagnostic: &AuthoringDiagnostic,
    lsp_diagnostics: &[Diagnostic],
) -> Vec<CodeActionOrCommand> {
    let Some(route) = diagnostic.resolved_route.as_deref() else {
        return Vec::new();
    };
    let Some(source_file) = source_file_for_new_route(route) else {
        return Vec::new();
    };
    let title = format!("Create page '{}'", source_file);
    let arguments = create_page_command_arguments(uri, route);

    vec![CodeActionOrCommand::CodeAction(CodeAction {
        title: title.clone(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(lsp_diagnostics_for_range(
            lsp_diagnostics,
            diagnostic.range(),
        )),
        edit: None,
        command: Some(Command {
            title,
            command: CREATE_PAGE_COMMAND.to_string(),
            arguments: Some(arguments),
        }),
        is_preferred: Some(true),
        ..CodeAction::default()
    })]
}

fn missing_anchor_code_actions(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    source_content: &str,
    diagnostic: &AuthoringDiagnostic,
    lsp_diagnostics: &[Diagnostic],
) -> Vec<CodeActionOrCommand> {
    let Some(target_route) = diagnostic.resolved_route.as_deref() else {
        return Vec::new();
    };
    let Some(target_page) = project.page_for_route(target_route) else {
        return Vec::new();
    };
    let Some(missing_fragment) = missing_anchor_fragment(&diagnostic.target) else {
        return Vec::new();
    };
    let Some(context) = target_context_for_diagnostic(source_content, diagnostic) else {
        return Vec::new();
    };
    let Some(fragment_range) =
        fragment_range_for_target_context(source_content, &context, missing_fragment)
    else {
        return Vec::new();
    };

    let mut actions = existing_anchor_code_actions(
        content_dir,
        target_page,
        diagnostic,
        lsp_diagnostics,
        fragment_range,
        missing_fragment,
    );
    if let Some(action) = create_anchor_code_action(
        content_dir,
        project,
        target_page,
        diagnostic,
        lsp_diagnostics,
        missing_fragment,
    ) {
        actions.push(action);
    }
    actions
}

fn existing_anchor_code_actions(
    content_dir: &Utf8Path,
    target_page: &AuthoringPage,
    diagnostic: &AuthoringDiagnostic,
    lsp_diagnostics: &[Diagnostic],
    fragment_range: Range,
    missing_fragment: &str,
) -> Vec<CodeActionOrCommand> {
    let Some(source_uri) = source_file_uri(content_dir, &diagnostic.source_file).ok() else {
        return Vec::new();
    };
    let mut headings = target_page.headings.iter().collect::<Vec<_>>();
    headings.sort_by(|left, right| {
        anchor_suggestion_score(missing_fragment, &right.id, &right.title)
            .cmp(&anchor_suggestion_score(
                missing_fragment,
                &left.id,
                &left.title,
            ))
            .then_with(|| left.id.cmp(&right.id))
    });

    headings
        .into_iter()
        .take(5)
        .enumerate()
        .filter(|(_, heading)| heading.id != missing_fragment)
        .map(|(idx, heading)| {
            let title = format!("Change anchor to '#{}'", heading.id);
            let edit = WorkspaceEdit::new(HashMap::from([(
                source_uri.clone(),
                vec![TextEdit::new(fragment_range, heading.id.clone())],
            )]));
            CodeActionOrCommand::CodeAction(CodeAction {
                title,
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(lsp_diagnostics_for_range(
                    lsp_diagnostics,
                    diagnostic.range(),
                )),
                edit: Some(edit),
                command: None,
                is_preferred: Some(idx == 0),
                ..CodeAction::default()
            })
        })
        .collect()
}

fn create_anchor_code_action(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
    diagnostic: &AuthoringDiagnostic,
    lsp_diagnostics: &[Diagnostic],
    missing_fragment: &str,
) -> Option<CodeActionOrCommand> {
    let target_content = project.source_contents.get(&target_page.source_file)?;
    let heading_edit =
        create_missing_anchor_heading_edit(target_page, target_content, missing_fragment)?;
    let target_uri = source_file_uri(content_dir, &target_page.source_file).ok()?;
    let edit = WorkspaceEdit::new(HashMap::from([(
        target_uri,
        vec![TextEdit::new(heading_edit.range, heading_edit.new_text)],
    )]));
    let title = format!("Create heading for '#{missing_fragment}'");

    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(lsp_diagnostics_for_range(
            lsp_diagnostics,
            diagnostic.range(),
        )),
        edit: Some(edit),
        command: None,
        is_preferred: Some(target_page.headings.is_empty()),
        ..CodeAction::default()
    }))
}

fn lsp_diagnostics_for_range(lsp_diagnostics: &[Diagnostic], range: Range) -> Vec<Diagnostic> {
    lsp_diagnostics
        .iter()
        .filter(|lsp_diagnostic| lsp_diagnostic.range == range)
        .cloned()
        .collect()
}

fn extract_page_code_action(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    uri: &Url,
    content: &str,
    selection: Range,
) -> Result<Option<CodeActionOrCommand>> {
    let Some(plan) = extract_page_plan(content_dir, project, uri, content, selection)? else {
        return Ok(None);
    };
    let title = format!("Extract selection to '{}'", plan.new_source_file);
    Ok(Some(CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::REFACTOR_EXTRACT),
        diagnostics: None,
        edit: Some(workspace_edit_for_extract_page(content_dir, &plan)?),
        command: None,
        is_preferred: Some(true),
        ..CodeAction::default()
    })))
}

fn link_hover_markdown(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
    reference: &MarkdownReference,
) -> String {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return format!("**Dodeca link**\n\nSpecial target: `{target}`");
    }

    if let Some(diagnostic) = diagnostic_for_reference(project, page, content, reference.clone()) {
        return format!(
            "**Dodeca link**\n\n{}\n\nTarget: `{}`",
            diagnostic.message, diagnostic.target
        );
    }

    let (target_without_fragment, fragment) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        let Some(route) = project.source_to_route.get(source_target) else {
            return format!("**Dodeca source link**\n\nSource not found: `{source_target}`");
        };
        let Some(target_page) = project.page_for_route(route) else {
            return format!("**Dodeca source link**\n\nRoute: `{route}`");
        };
        return page_link_hover_markdown(project, target_page, fragment);
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return match project.static_target_path(&page.source_file, target_without_fragment) {
            Some(path) => format!(
                "**Dodeca static asset**\n\nURL: `{target_without_fragment}`\n\nFile: `{path}`"
            ),
            None => format!(
                "**Dodeca static asset**\n\nStatic asset not found: `{target_without_fragment}`"
            ),
        };
    }

    let target_route = route_for_link_target(project, page, target_without_fragment);
    let Some(target_page) = project.page_for_route(&target_route) else {
        return format!("**Dodeca route**\n\nRoute not found: `{target_route}`");
    };
    page_link_hover_markdown(project, target_page, fragment)
}

fn page_link_hover_markdown(
    project: &AuthoringProject,
    page: &AuthoringPage,
    fragment: Option<&str>,
) -> String {
    let mut sections = vec![format!("**Dodeca {}**", page.title)];

    if let Some(description) = page
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        sections.push(description.to_string());
    }

    if let Some(content) = project.source_contents.get(&page.source_file)
        && let Some(excerpt) = markdown_content_excerpt(content)
    {
        sections.push(excerpt);
    }

    if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
        match project.heading_for_route(&page.route, fragment) {
            Some(heading) => sections.push(format!(
                "**Heading**: H{} `{}` (`#{}`)",
                heading.level, heading.title, heading.id
            )),
            None => sections.push(format!("**Heading not found**: `#{fragment}`")),
        }
    }

    sections.push(page_hover_metadata_table(page));
    sections.join("\n\n")
}

fn frontmatter_hover_markdown(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
    backlink_count: usize,
) -> String {
    let kind = match page.kind {
        AuthoringPageKind::Page => "page",
        AuthoringPageKind::Section => "section",
    };
    let mut sections = vec![format!("**Dodeca {kind}: {}**", page.title)];

    if let Some(description) = page
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        sections.push(description.to_string());
    }

    if let Some(excerpt) = markdown_content_excerpt(content) {
        sections.push(excerpt);
    }

    sections.push(frontmatter_hover_metadata_table(page, backlink_count));
    sections.push(build_provenance_hover_table(project, page, content));
    sections.join("\n\n")
}

fn page_hover_metadata_table(page: &AuthoringPage) -> String {
    format!(
        "| route | source | template | output |\n| --- | --- | --- | --- |\n| `{}` | `{}` | `{}` | `{}` |",
        page.route, page.source_file, page.template, page.output_path
    )
}

fn frontmatter_hover_metadata_table(page: &AuthoringPage, backlink_count: usize) -> String {
    format!(
        "| route | source | headings | backlinks |\n| --- | --- | ---: | ---: |\n| `{}` | `{}` | `{}` | `{}` |",
        page.route,
        page.source_file,
        page.heading_ids.len(),
        backlink_count
    )
}

fn build_provenance_hover_table(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
) -> String {
    let mut rows = vec![
        "| build | value |".to_string(),
        "| --- | --- |".to_string(),
        format!("| transforms | `{}` |", page_transform_chain(page)),
    ];

    let templates = template_dependency_names(project, &page.template);
    if !templates.is_empty() {
        rows.push(format!("| templates | `{}` |", templates.join("`, `")));
    }

    let assets = static_asset_references(project, page, content);
    if !assets.is_empty() {
        rows.push(format!("| static assets | `{}` |", assets.join("`, `")));
    }

    if !project.data_keys.is_empty() {
        rows.push(format!(
            "| data keys | `{}` |",
            project.data_keys.join("`, `")
        ));
    }

    rows.join("\n")
}

fn markdown_content_excerpt(content: &str) -> Option<String> {
    let body = markdown_body_without_frontmatter(content).trim();
    if body.is_empty() {
        return None;
    }

    let mut excerpt = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");
    excerpt = collapse_whitespace(&excerpt);

    const MAX_EXCERPT_CHARS: usize = 260;
    if excerpt.chars().count() > MAX_EXCERPT_CHARS {
        excerpt = excerpt
            .chars()
            .take(MAX_EXCERPT_CHARS.saturating_sub(3))
            .collect::<String>();
        excerpt.push_str("...");
    }

    Some(format!("> {excerpt}"))
}

fn markdown_body_without_frontmatter(content: &str) -> &str {
    if !content.starts_with("+++\n") {
        return content;
    }

    let rest = &content[4..];
    let Some(end) = rest.find("\n+++") else {
        return content;
    };
    let after_delimiter = 4 + end + "\n+++".len();
    content[after_delimiter..]
        .strip_prefix('\n')
        .unwrap_or(&content[after_delimiter..])
}

fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn page_transform_chain(page: &AuthoringPage) -> &'static str {
    match page.kind {
        AuthoringPageKind::Page => "markdown -> page template -> html postprocess -> output",
        AuthoringPageKind::Section => "markdown -> section template -> html postprocess -> output",
    }
}

fn template_dependency_names(project: &AuthoringProject, root_template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    collect_template_dependency_names(project, root_template, &mut seen, &mut names);
    names
}

fn collect_template_dependency_names(
    project: &AuthoringProject,
    template_file: &str,
    seen: &mut HashSet<String>,
    names: &mut Vec<String>,
) {
    if !seen.insert(template_file.to_string()) {
        return;
    }
    if !project.template_contents.contains_key(template_file) {
        return;
    }
    names.push(template_file.to_string());
    let Some(content) = project.template_contents.get(template_file) else {
        return;
    };
    let Ok(template) = TemplateParser::new(template_file, content.as_str()).parse() else {
        return;
    };
    for dependency in template_path_dependencies(&template.body) {
        collect_template_dependency_names(project, &dependency, seen, names);
    }
}

fn template_path_dependencies(nodes: &[Node]) -> Vec<String> {
    let mut dependencies = Vec::new();
    collect_template_path_dependencies(nodes, &mut dependencies);
    dependencies
}

fn collect_template_path_dependencies(nodes: &[Node], dependencies: &mut Vec<String>) {
    for node in nodes {
        match node {
            Node::Extends(node) => dependencies.push(node.path.value.clone()),
            Node::Include(node) => dependencies.push(node.path.value.clone()),
            Node::Import(node) => dependencies.push(node.path.value.clone()),
            Node::If(node) => {
                collect_template_path_dependencies(&node.then_body, dependencies);
                for branch in &node.elif_branches {
                    collect_template_path_dependencies(&branch.body, dependencies);
                }
                if let Some(body) = &node.else_body {
                    collect_template_path_dependencies(body, dependencies);
                }
            }
            Node::For(node) => {
                collect_template_path_dependencies(&node.body, dependencies);
                if let Some(body) = &node.else_body {
                    collect_template_path_dependencies(body, dependencies);
                }
            }
            Node::Block(node) => collect_template_path_dependencies(&node.body, dependencies),
            Node::Macro(node) => collect_template_path_dependencies(&node.body, dependencies),
            Node::Text(_)
            | Node::Print(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

fn static_asset_references(
    project: &AuthoringProject,
    page: &AuthoringPage,
    content: &str,
) -> Vec<String> {
    let mut assets = markdown_references(content)
        .into_iter()
        .filter_map(|reference| {
            let target = reference.target.as_str();
            let (target_without_fragment, _) = split_fragment(target);
            ((reference.kind == MarkdownReferenceKind::Image
                || is_likely_static_file(target_without_fragment))
                && project.static_target_exists(&page.source_file, target_without_fragment))
            .then(|| target_without_fragment.trim_start_matches('/').to_string())
        })
        .collect::<Vec<_>>();
    assets.sort();
    assets.dedup();
    assets
}

fn markdown_hover(markdown: String, range: Range) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(range),
    }
}

fn template_semantic_hover(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Option<Hover> {
    let offset = position_to_byte_offset(content, position)?;
    let index = project.template_semantics.get(template_file)?;
    if let Some(reference) = index.reference_at_offset(offset) {
        let markdown = match reference.kind {
            TemplateReferenceKind::Field => template_field_hover_markdown(index, reference, offset),
            TemplateReferenceKind::Filter => {
                template_item_hover_markdown(&reference.name, template_filter_info(&reference.name))
            }
            TemplateReferenceKind::Test => {
                template_item_hover_markdown(&reference.name, template_test_info(&reference.name))
            }
            TemplateReferenceKind::Macro => template_item_hover_markdown(
                &reference.name,
                TemplateItemInfo {
                    detail: "Gingembre macro",
                    documentation: "Macro callable from this Gingembre template expression.",
                },
            ),
            TemplateReferenceKind::Variable
            | TemplateReferenceKind::Function
            | TemplateReferenceKind::MacroNamespace => {
                let symbol = reference.symbol_id.and_then(|id| index.symbols.get(id))?;
                template_symbol_hover_markdown(symbol)
            }
        };
        return Some(markdown_hover(
            markdown,
            byte_range_to_lsp_range(
                content,
                reference.span.offset(),
                reference.span.offset() + reference.span.len(),
            ),
        ));
    }

    let symbol = index.symbol_at_offset(offset)?;
    let span = symbol.span?;
    Some(markdown_hover(
        template_symbol_hover_markdown(symbol),
        byte_range_to_lsp_range(content, span.offset(), span.offset() + span.len()),
    ))
}

fn template_semantic_definition(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    template_file: &str,
    position: Position,
) -> Option<Location> {
    let content = project.template_contents.get(template_file)?;
    let offset = position_to_byte_offset(content, position)?;
    let index = project.template_semantics.get(template_file)?;
    let symbol = index.symbol_for_offset(offset)?;
    let span = symbol.span?;
    Some(Location {
        uri: Url::from_file_path(
            content_dir
                .parent()
                .unwrap_or(content_dir)
                .join("templates")
                .join(template_file)
                .as_std_path(),
        )
        .ok()?,
        range: byte_range_to_lsp_range(content, span.offset(), span.offset() + span.len()),
    })
}

fn template_semantic_references(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Vec<Location> {
    let Some(offset) = position_to_byte_offset(content, position) else {
        return Vec::new();
    };
    let Some(index) = project.template_semantics.get(template_file) else {
        return Vec::new();
    };
    let Some(symbol) = index.symbol_for_offset(offset) else {
        return Vec::new();
    };
    let Some(uri) = Url::from_file_path(
        content_dir
            .parent()
            .unwrap_or(content_dir)
            .join("templates")
            .join(template_file)
            .as_std_path(),
    )
    .ok() else {
        return Vec::new();
    };
    let mut locations = Vec::new();
    if let Some(span) = symbol.span {
        locations.push(Location {
            uri: uri.clone(),
            range: byte_range_to_lsp_range(content, span.offset(), span.offset() + span.len()),
        });
    }
    locations.extend(
        index
            .references_to_symbol(symbol.id)
            .into_iter()
            .map(|reference| Location {
                uri: uri.clone(),
                range: byte_range_to_lsp_range(
                    content,
                    reference.span.offset(),
                    reference.span.offset() + reference.span.len(),
                ),
            }),
    );
    locations
}

fn template_semantic_prepare_rename(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Option<PrepareRenameResponse> {
    let offset = position_to_byte_offset(content, position)?;
    let index = project.template_semantics.get(template_file)?;
    let symbol = index.symbol_for_offset(offset)?;
    if !template_symbol_can_rename(symbol) {
        return None;
    }
    let span = symbol.span?;
    Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: byte_range_to_lsp_range(content, span.offset(), span.offset() + span.len()),
        placeholder: symbol.name.clone(),
    })
}

fn template_semantic_rename_workspace_edit(
    uri: &Url,
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    if !is_valid_template_rename_name(new_name) {
        return Ok(None);
    }
    let Some(offset) = position_to_byte_offset(content, position) else {
        return Ok(None);
    };
    let Some(index) = project.template_semantics.get(template_file) else {
        return Ok(None);
    };
    let Some(symbol) = index.symbol_for_offset(offset) else {
        return Ok(None);
    };
    if !template_symbol_can_rename(symbol) {
        return Ok(None);
    }
    let mut edits = Vec::new();
    if let Some(span) = symbol.span {
        edits.push(TextEdit {
            range: byte_range_to_lsp_range(content, span.offset(), span.offset() + span.len()),
            new_text: new_name.to_string(),
        });
    }
    edits.extend(
        index
            .references_to_symbol(symbol.id)
            .into_iter()
            .map(|reference| TextEdit {
                range: byte_range_to_lsp_range(
                    content,
                    reference.span.offset(),
                    reference.span.offset() + reference.span.len(),
                ),
                new_text: new_name.to_string(),
            }),
    );
    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

fn template_symbol_can_rename(symbol: &TemplateSymbol) -> bool {
    symbol.span.is_some()
        && matches!(
            symbol.kind,
            TemplateSymbolKind::SetBinding
                | TemplateSymbolKind::LoopBinding
                | TemplateSymbolKind::MacroParam
                | TemplateSymbolKind::ImportAlias
                | TemplateSymbolKind::Macro
        )
}

fn is_valid_template_rename_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn template_symbol_hover_markdown(symbol: &TemplateSymbol) -> String {
    let info = template_symbol_info(symbol);
    format!(
        "**{}** `{}`\n\n{}",
        info.detail, symbol.name, info.documentation
    )
}

fn template_field_hover_markdown(
    index: &TemplateSemanticIndex,
    reference: &gingembre::semantic::TemplateReference,
    offset: usize,
) -> String {
    let path = reference
        .path
        .split_last()
        .map(|(_, base)| base.to_vec())
        .unwrap_or_default();
    let path = resolve_template_expression_path(index, &path, offset, 0).unwrap_or(path);
    let info = template_field_info(&path, &reference.name);
    template_item_hover_markdown(&reference.name, info)
}

fn template_item_hover_markdown(name: &str, info: TemplateItemInfo) -> String {
    format!("**{}** `{}`\n\n{}", info.detail, name, info.documentation)
}

fn template_semantic_tokens(index: &TemplateSemanticIndex, content: &str) -> Vec<SemanticToken> {
    let mut spans = index.tokens.clone();
    spans.sort_by_key(|token| (token.span.offset(), token.span.len()));
    spans.dedup_by_key(|token| (token.span.offset(), token.span.len(), token.kind));

    let mut result = Vec::new();
    let mut previous_line = 0;
    let mut previous_start = 0;
    for token in spans {
        let start = token.span.offset();
        let end = start.saturating_add(token.span.len());
        if start >= content.len() || end > content.len() || content[start..end].contains('\n') {
            continue;
        }
        let (line, column) = byte_to_line_column(content, start);
        let line = line.saturating_sub(1);
        let start_character = column.saturating_sub(1);
        let delta_line = line.saturating_sub(previous_line);
        let delta_start = if delta_line == 0 {
            start_character.saturating_sub(previous_start)
        } else {
            start_character
        };
        result.push(SemanticToken {
            delta_line,
            delta_start,
            length: content[start..end].chars().count() as u32,
            token_type: template_semantic_token_type(token.kind),
            token_modifiers_bitset: 0,
        });
        previous_line = line;
        previous_start = start_character;
    }
    result
}

fn template_semantic_tokens_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: vec![
            SemanticTokenType::VARIABLE,
            SemanticTokenType::PARAMETER,
            SemanticTokenType::PROPERTY,
            SemanticTokenType::FUNCTION,
            SemanticTokenType::MACRO,
            SemanticTokenType::STRING,
            SemanticTokenType::NUMBER,
            SemanticTokenType::KEYWORD,
        ],
        token_modifiers: vec![SemanticTokenModifier::DEFAULT_LIBRARY],
    }
}

fn template_semantic_token_type(kind: TemplateSemanticTokenKind) -> u32 {
    match kind {
        TemplateSemanticTokenKind::Variable => TEMPLATE_SEMANTIC_TOKEN_VARIABLE,
        TemplateSemanticTokenKind::Parameter => TEMPLATE_SEMANTIC_TOKEN_PARAMETER,
        TemplateSemanticTokenKind::Property => TEMPLATE_SEMANTIC_TOKEN_PROPERTY,
        TemplateSemanticTokenKind::Function => TEMPLATE_SEMANTIC_TOKEN_FUNCTION,
        TemplateSemanticTokenKind::Macro => TEMPLATE_SEMANTIC_TOKEN_MACRO,
        TemplateSemanticTokenKind::String => TEMPLATE_SEMANTIC_TOKEN_STRING,
        TemplateSemanticTokenKind::Number => TEMPLATE_SEMANTIC_TOKEN_NUMBER,
        TemplateSemanticTokenKind::Keyword => TEMPLATE_SEMANTIC_TOKEN_KEYWORD,
    }
}

#[allow(deprecated)]
fn document_symbol_for_page(page: &AuthoringPage, content: &str) -> DocumentSymbol {
    let range = full_document_range(content);
    let selection_range = frontmatter_lsp_range(content).unwrap_or_else(|| one_line_range(0));
    let heading_lines = markdown_headings(content);
    let children = page
        .headings
        .iter()
        .map(|heading| {
            let line = heading_lines
                .iter()
                .find(|candidate| candidate.id == heading.id)
                .map(|candidate| candidate.line.saturating_sub(1))
                .unwrap_or(0);
            DocumentSymbol {
                name: heading.title.clone(),
                detail: Some(format!("#{} on {}", heading.id, page.route)),
                kind: SymbolKind::STRING,
                tags: None,
                deprecated: None,
                range: one_line_range(line),
                selection_range: one_line_range(line),
                children: None,
            }
        })
        .collect::<Vec<_>>();

    DocumentSymbol {
        name: page.title.clone(),
        detail: Some(format!("{} {}", page.route, page.source_file)),
        kind: match page.kind {
            AuthoringPageKind::Page => SymbolKind::FILE,
            AuthoringPageKind::Section => SymbolKind::MODULE,
        },
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: (!children.is_empty()).then_some(children),
    }
}

fn workspace_symbols_for_project(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    query: &str,
) -> Vec<SymbolInformation> {
    let mut symbols = Vec::new();
    for page in &project.pages {
        if workspace_page_matches(page, query)
            && let Some(symbol) = symbol_information_for_page(content_dir, project, page)
        {
            symbols.push(symbol);
        }

        for heading in &page.headings {
            if workspace_heading_matches(page, heading, query)
                && let Some(symbol) =
                    symbol_information_for_heading(content_dir, project, page, &heading.id)
            {
                symbols.push(symbol);
            }
        }
    }

    symbols.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.location.uri.as_str().cmp(b.location.uri.as_str()))
            .then_with(|| position_cmp(a.location.range.start, b.location.range.start))
    });
    symbols
}

fn workspace_page_matches(page: &AuthoringPage, query: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        page.title, page.route, page.source_file, page.template, page.output_path
    );
    fuzzy_contains(&haystack, query)
}

fn workspace_heading_matches(
    page: &AuthoringPage,
    heading: &crate::authoring_model::AuthoringHeading,
    query: &str,
) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        heading.title, heading.id, page.title, page.route, page.source_file
    );
    fuzzy_contains(&haystack, query)
}

fn fuzzy_contains(haystack: &str, query: &str) -> bool {
    let haystack = haystack.to_lowercase();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|part| haystack.contains(&part))
}

#[allow(deprecated)]
fn symbol_information_for_page(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    page: &AuthoringPage,
) -> Option<SymbolInformation> {
    Some(SymbolInformation {
        name: page.title.clone(),
        kind: match page.kind {
            AuthoringPageKind::Page => SymbolKind::FILE,
            AuthoringPageKind::Section => SymbolKind::MODULE,
        },
        tags: None,
        deprecated: None,
        location: location_for_page(content_dir, project, page)?,
        container_name: Some(format!("{} {}", page.route, page.source_file)),
    })
}

#[allow(deprecated)]
fn symbol_information_for_heading(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    page: &AuthoringPage,
    heading_id: &str,
) -> Option<SymbolInformation> {
    let heading = page
        .headings
        .iter()
        .find(|heading| heading.id == heading_id)?;
    Some(SymbolInformation {
        name: heading.title.clone(),
        kind: SymbolKind::STRING,
        tags: None,
        deprecated: None,
        location: location_for_page_heading(content_dir, project, page, heading_id)?,
        container_name: Some(format!("{} {}", page.title, page.route)),
    })
}

fn location_for_page(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    page: &AuthoringPage,
) -> Option<Location> {
    let uri = Url::from_file_path(content_dir.join(&page.source_file)).ok()?;
    let content = project.source_contents.get(&page.source_file)?;
    let range = frontmatter_lsp_range(content).unwrap_or_else(|| one_line_range(0));
    Some(Location { uri, range })
}

fn location_for_page_heading(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    page: &AuthoringPage,
    heading_id: &str,
) -> Option<Location> {
    let uri = Url::from_file_path(content_dir.join(&page.source_file)).ok()?;
    let content = project.source_contents.get(&page.source_file)?;
    let line = markdown_headings(content)
        .into_iter()
        .find(|heading| heading.id == heading_id)
        .map(|heading| heading.line.saturating_sub(1))
        .unwrap_or(0);
    Some(Location {
        uri,
        range: one_line_range(line),
    })
}

fn full_document_range(content: &str) -> Range {
    let (line, character) = byte_to_line_column(content, content.len());
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: line.saturating_sub(1),
            character: character.saturating_sub(1),
        },
    }
}

fn one_line_range(line: u32) -> Range {
    Range {
        start: Position { line, character: 0 },
        end: Position { line, character: 0 },
    }
}

fn definition_for_reference(
    dirs: &AuthoringDirs,
    project: &AuthoringProject,
    page: &AuthoringPage,
    reference: &MarkdownReference,
) -> Result<Option<Location>> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return Ok(None);
    }

    let (target_without_fragment, fragment) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        let Some(source_file) = project
            .source_to_route
            .get(source_target)
            .map(|_| source_target)
        else {
            return Ok(None);
        };
        let path = dirs.content_dir.join(source_file);
        return location_for_source_path(&path, fragment);
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return Ok(location_for_static_target(
            project,
            &page.source_file,
            target_without_fragment,
        ));
    }

    let target_route = route_for_link_target(project, page, target_without_fragment);

    let Some(source_file) = project.source_file_for_route(&target_route) else {
        return Ok(None);
    };
    let path = dirs.content_dir.join(source_file);
    location_for_source_path(&path, fragment)
}

fn references_to_page(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };

        for reference in markdown_references(content) {
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
                continue;
            }
            if let Some(location) =
                location_for_markdown_reference(content_dir, &page.source_file, content, &reference)
            {
                locations.push(location);
            }
        }
    }

    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            let Some(target_route) = rendered_href_target_route(project, source_page, &href.href)
            else {
                continue;
            };
            if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
                continue;
            }
            let Some(origin) = &href.origin else {
                continue;
            };
            if let Some(location) = location_for_rendered_href_origin(content_dir, project, origin)
            {
                locations.push(location);
            }
        }
    }

    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    locations.dedup_by(|left, right| left.uri == right.uri && left.range == right.range);
    Ok(locations)
}

fn location_for_rendered_href_origin(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    origin: &RenderedHrefOrigin,
) -> Option<Location> {
    let (path, content) = match &origin.path {
        AuthoringInputPath::Source(source_file) => (
            content_dir.join(source_file),
            project.source_contents.get(source_file)?,
        ),
        AuthoringInputPath::Template(template_file) => (
            content_dir
                .parent()
                .unwrap_or(content_dir)
                .join("templates")
                .join(template_file),
            project.template_contents.get(template_file)?,
        ),
        AuthoringInputPath::Sass(_)
        | AuthoringInputPath::Static(_)
        | AuthoringInputPath::Dist(_)
        | AuthoringInputPath::Data(_) => return None,
    };
    Some(Location {
        uri: Url::from_file_path(path.as_std_path()).ok()?,
        range: byte_range_to_lsp_range(content, origin.byte_start, origin.byte_end),
    })
}

fn references_to_heading(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
    heading_id: &str,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };

        for reference in markdown_references(content) {
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
                continue;
            }

            let (_, fragment) = split_fragment(&reference.target);
            if fragment != Some(heading_id) {
                continue;
            }

            if let Some(location) =
                location_for_markdown_reference(content_dir, &page.source_file, content, &reference)
            {
                locations.push(location);
            }
        }
    }

    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    Ok(locations)
}

fn rename_heading_workspace_edit(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
    target_source_file: &str,
    target_content: &str,
    target: &HeadingRenameTarget,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('\n') || new_name.contains('\r') {
        return Ok(None);
    }

    let Some(new_heading_id) = heading_id_after_rename(target_content, target, new_name) else {
        return Ok(None);
    };
    if new_heading_id == target.heading_id && new_name == target.title {
        return Ok(None);
    }

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    let target_uri = Url::from_file_path(content_dir.join(target_source_file))
        .map_err(|_| eyre!("could not convert source file to URI: {target_source_file}"))?;
    changes
        .entry(target_uri)
        .or_default()
        .push(TextEdit::new(target.title_range, new_name.to_string()));

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        let Some(uri) = Url::from_file_path(content_dir.join(&page.source_file)).ok() else {
            continue;
        };

        for context in markdown_target_contexts(content) {
            let reference = MarkdownReference {
                kind: context.kind,
                target: context.target.clone(),
                byte_start: context.byte_start,
                byte_end: context.byte_end,
            };
            let Some(target_route) = reference_target_route(project, page, &reference) else {
                continue;
            };
            if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
                continue;
            }

            let (_, fragment) = split_fragment(&context.target);
            if fragment != Some(target.heading_id.as_str()) {
                continue;
            }

            let Some(range) =
                fragment_range_for_target_context(content, &context, &target.heading_id)
            else {
                continue;
            };
            changes
                .entry(uri.clone())
                .or_default()
                .push(TextEdit::new(range, new_heading_id.clone()));
        }
    }

    for edits in changes.values_mut() {
        edits.sort_by(|a, b| {
            position_cmp(a.range.start, b.range.start)
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
    }

    Ok(Some(WorkspaceEdit::new(changes)))
}

fn heading_id_after_rename(
    content: &str,
    target: &HeadingRenameTarget,
    new_name: &str,
) -> Option<String> {
    let start = lsp_position_to_byte_offset(content, target.title_range.start)?;
    let end = lsp_position_to_byte_offset(content, target.title_range.end)?;
    let mut renamed = content.to_string();
    renamed.replace_range(start..end, new_name);

    markdown_headings(&renamed)
        .into_iter()
        .find(|heading| heading.line == target.line)
        .map(|heading| heading.id)
}

fn fragment_range_for_target_context(
    content: &str,
    context: &MarkdownTargetContext,
    expected_fragment: &str,
) -> Option<Range> {
    let fragment_start_in_target = context.target.find('#')? + 1;
    let fragment_end_in_target = context.target[fragment_start_in_target..]
        .find('?')
        .map(|idx| fragment_start_in_target + idx)
        .unwrap_or(context.target.len());
    if &context.target[fragment_start_in_target..fragment_end_in_target] != expected_fragment {
        return None;
    }

    Some(byte_range_to_lsp_range(
        content,
        context.byte_start + fragment_start_in_target,
        context.byte_start + fragment_end_in_target,
    ))
}

fn missing_anchor_fragment(target: &str) -> Option<&str> {
    split_fragment(target)
        .1
        .filter(|fragment| !fragment.is_empty())
}

fn target_context_for_diagnostic(
    content: &str,
    diagnostic: &AuthoringDiagnostic,
) -> Option<MarkdownTargetContext> {
    markdown_target_contexts(content)
        .into_iter()
        .find(|context| {
            context.target == diagnostic.target
                && diagnostic.byte_start <= context.byte_start
                && context.byte_end <= diagnostic.byte_end
        })
}

fn anchor_suggestion_score(fragment: &str, heading_id: &str, heading_title: &str) -> usize {
    let fragment = fragment.to_lowercase();
    let heading_id = heading_id.to_lowercase();
    let heading_title = marq::slugify(heading_title);

    if heading_id == fragment {
        return usize::MAX;
    }
    let fragment_leaf = fragment.rsplit("--").next().unwrap_or(fragment.as_str());
    let heading_leaf = heading_id
        .rsplit("--")
        .next()
        .unwrap_or(heading_id.as_str());
    let mut score = common_prefix_len(&fragment, &heading_id)
        + common_prefix_len(fragment_leaf, heading_leaf) * 3;
    if heading_id.contains(&fragment) || fragment.contains(&heading_id) {
        score += 100;
    }
    if heading_title.contains(fragment_leaf) || fragment_leaf.contains(&heading_title) {
        score += 50;
    }
    score
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MissingAnchorHeadingEdit {
    range: Range,
    new_text: String,
}

fn create_missing_anchor_heading_edit(
    page: &AuthoringPage,
    content: &str,
    missing_fragment: &str,
) -> Option<MissingAnchorHeadingEdit> {
    let local_headings = markdown_headings(content);
    let (insert_position, level, title) =
        missing_anchor_heading_insertion(page, content, &local_headings, missing_fragment)?;
    Some(MissingAnchorHeadingEdit {
        range: Range {
            start: insert_position,
            end: insert_position,
        },
        new_text: format!("\n\n{} {}\n", "#".repeat(level as usize), title),
    })
}

fn missing_anchor_heading_insertion(
    page: &AuthoringPage,
    content: &str,
    local_headings: &[MarkdownHeading],
    missing_fragment: &str,
) -> Option<(Position, u8, String)> {
    let (parent_id, leaf_slug) = missing_anchor_parent_and_leaf(missing_fragment);
    let title = title_from_slug(leaf_slug)?;

    if let Some(parent_id) = parent_id
        && let Some(parent_heading) = local_headings.iter().find(|heading| {
            heading.id == parent_id && page.heading_ids.iter().any(|id| id == &heading.id)
        })
    {
        return Some((
            insertion_position_after_line(content, parent_heading.line),
            parent_heading.level.saturating_add(1).min(6),
            title,
        ));
    }

    let (line, character) = byte_to_line_column(content, content.len());
    Some((
        Position {
            line: line.saturating_sub(1),
            character: character.saturating_sub(1),
        },
        1,
        title,
    ))
}

fn missing_anchor_parent_and_leaf(fragment: &str) -> (Option<&str>, &str) {
    match fragment.rfind("--") {
        Some(idx) => (Some(&fragment[..idx]), &fragment[idx + 2..]),
        None => (None, fragment),
    }
}

fn title_from_slug(slug: &str) -> Option<String> {
    let words = slug
        .split('-')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>();
    (!words.is_empty()).then(|| words.join(" "))
}

fn insertion_position_after_line(content: &str, one_based_line: u32) -> Position {
    let line_idx = one_based_line.saturating_sub(1) as usize;
    let byte = content
        .split_inclusive('\n')
        .take(line_idx + 1)
        .map(str::len)
        .sum::<usize>();
    let (line, column) = byte_to_line_column(content, byte);
    Position {
        line: line.saturating_sub(1),
        character: column.saturating_sub(1),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExtractPagePlan {
    source_file: String,
    new_source_file: String,
    new_route: String,
    title: String,
    new_content: String,
    replacement: String,
    selection: Range,
}

fn extract_page_plan(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    uri: &Url,
    content: &str,
    selection: Range,
) -> Result<Option<ExtractPagePlan>> {
    if selection.start == selection.end {
        return Ok(None);
    }
    if let Some(frontmatter_range) = frontmatter_lsp_range(content)
        && ranges_overlap(&frontmatter_range, &selection)
    {
        return Ok(None);
    }

    let path = lsp_file_uri_to_utf8_path(uri)?;
    let source_file = source_file_for_path(content_dir, &path)?;
    let Some(page) = project.page_for_source_file(&source_file) else {
        return Ok(None);
    };
    let Some((start, end)) = lsp_range_to_byte_range(content, selection) else {
        return Ok(None);
    };
    if start >= end {
        return Ok(None);
    }
    let selected = &content[start..end];
    if selected.trim().is_empty() {
        return Ok(None);
    }

    let extracted = extracted_page_content(selected);
    let new_source_file =
        unique_extracted_source_file(content_dir, project, &source_file, &extracted.slug)?;
    let new_route = SourcePath::new(new_source_file.clone())
        .to_route()
        .to_string();
    let target = route_completion_text(page, "", &new_route);
    let replacement = format!("[{}]({target})", extracted.title);
    let new_content = format!("{}{}", page_frontmatter(&extracted.title), extracted.body);

    Ok(Some(ExtractPagePlan {
        source_file,
        new_source_file,
        new_route,
        title: extracted.title,
        new_content,
        replacement,
        selection,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExtractedPageContent {
    title: String,
    slug: String,
    body: String,
}

fn extracted_page_content(selected: &str) -> ExtractedPageContent {
    if let Some((title, body)) = extract_leading_heading(selected) {
        let slug = marq::slugify(&title);
        return ExtractedPageContent {
            title,
            slug,
            body: normalize_extracted_body(body),
        };
    }

    let title = title_from_selection(selected);
    let slug = marq::slugify(&title);
    ExtractedPageContent {
        title,
        slug,
        body: normalize_extracted_body(selected),
    }
}

fn extract_leading_heading(selected: &str) -> Option<(String, &str)> {
    let leading_len = selected.len() - selected.trim_start_matches([' ', '\t', '\n', '\r']).len();
    let after_leading = &selected[leading_len..];
    let line_end = after_leading.find('\n').unwrap_or(after_leading.len());
    let first_line = &after_leading[..line_end];
    let title = title_from_atx_heading_line(first_line)?;
    let body_start = leading_len + line_end + usize::from(line_end < after_leading.len());
    Some((title, &selected[body_start..]))
}

fn title_from_atx_heading_line(line: &str) -> Option<String> {
    let line = line.trim();
    let marker_len = line.chars().take_while(|ch| *ch == '#').count();
    if marker_len == 0 || marker_len > 6 {
        return None;
    }
    let rest = &line[marker_len..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let mut title = rest.trim().to_string();
    let trimmed_hashes = title.trim_end_matches('#').trim_end().to_string();
    if !trimmed_hashes.is_empty() {
        title = trimmed_hashes;
    }
    (!title.is_empty()).then_some(title)
}

fn title_from_selection(selected: &str) -> String {
    let text = selected
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("Extracted page");
    let words = text
        .split_whitespace()
        .take(6)
        .map(|word| {
            word.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '#' | '*' | '_' | '`' | '[' | ']' | '(' | ')' | ':' | ',' | '.' | '!' | '?'
                )
            })
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        "Extracted page".to_string()
    } else {
        words.join(" ")
    }
}

fn normalize_extracted_body(body: &str) -> String {
    let body = body.trim_matches(|ch| matches!(ch, '\n' | '\r'));
    if body.trim().is_empty() {
        "\n".to_string()
    } else {
        format!("\n{}\n", body.trim())
    }
}

fn unique_extracted_source_file(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    source_file: &str,
    slug: &str,
) -> Result<String> {
    let source_parent = Utf8Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let base_slug = if slug.is_empty() {
        "extracted-page"
    } else {
        slug
    };
    for idx in 0..1000 {
        let slug = if idx == 0 {
            base_slug.to_string()
        } else {
            format!("{base_slug}-{idx}")
        };
        let candidate = source_parent.join(format!("{slug}.md")).to_string();
        if !project.source_to_route.contains_key(&candidate)
            && !content_dir.join(&candidate).exists()
        {
            return Ok(candidate);
        }
    }
    Err(eyre!(
        "could not find available source file for extracted page"
    ))
}

fn workspace_edit_for_extract_page(
    content_dir: &Utf8Path,
    plan: &ExtractPagePlan,
) -> Result<WorkspaceEdit> {
    let source_uri = source_file_uri(content_dir, &plan.source_file)?;
    let new_uri = source_file_uri(content_dir, &plan.new_source_file)?;
    Ok(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Operations(vec![
            DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                uri: new_uri.clone(),
                options: Some(CreateFileOptions {
                    overwrite: Some(false),
                    ignore_if_exists: Some(false),
                }),
                annotation_id: None,
            })),
            DocumentChangeOperation::Edit(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: new_uri,
                    version: None,
                },
                edits: vec![OneOf::Left(TextEdit::new(
                    Range {
                        start: Position {
                            line: 0,
                            character: 0,
                        },
                        end: Position {
                            line: 0,
                            character: 0,
                        },
                    },
                    plan.new_content.clone(),
                ))],
            }),
            DocumentChangeOperation::Edit(TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    uri: source_uri,
                    version: None,
                },
                edits: vec![OneOf::Left(TextEdit::new(
                    plan.selection,
                    plan.replacement.clone(),
                ))],
            }),
        ])),
        change_annotations: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRouteRenamePlan {
    old_route: String,
    new_route: String,
    old_source_file: String,
    new_source_file: String,
    text_edits: Vec<PageRouteTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRouteTextEdit {
    path: AuthoringInputPath,
    range: Range,
    new_target: String,
}

fn rename_page_route_workspace_edit(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    let Some(plan) = page_route_rename_plan(content_dir, project, target_page, new_name)? else {
        return Ok(None);
    };
    Ok(Some(workspace_edit_for_page_route_rename(
        content_dir,
        &plan,
    )?))
}

fn page_route_rename_plan(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_page: &AuthoringPage,
    new_name: &str,
) -> Result<Option<PageRouteRenamePlan>> {
    let Some(target) = page_route_rename_target(target_page, new_name) else {
        return Ok(None);
    };
    if target.route == target_page.route && target.source_file == target_page.source_file {
        return Ok(None);
    }
    if project
        .source_file_for_route(&target.route)
        .is_some_and(|source_file| source_file != target_page.source_file)
    {
        return Ok(None);
    }
    if project.source_to_route.contains_key(&target.source_file)
        && target.source_file != target_page.source_file
    {
        return Ok(None);
    }
    if content_dir.join(&target.source_file).exists()
        && target.source_file != target_page.source_file
    {
        return Ok(None);
    }

    let mut text_edits = Vec::new();
    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };

        for context in markdown_target_contexts(content) {
            if let Some(edit) = page_route_link_edit(project, page, target_page, &target, context) {
                text_edits.push(edit);
            }
        }
    }
    for (source_route, hrefs) in &project.rendered_hrefs_by_route {
        let Some(source_page) = project.page_for_route(source_route) else {
            continue;
        };
        for href in hrefs {
            if let Some(edit) =
                page_route_rendered_href_edit(project, source_page, target_page, &target, href)
            {
                text_edits.push(edit);
            }
        }
    }

    text_edits.sort_by(|a, b| {
        page_route_text_edit_sort_key(&a.path)
            .cmp(&page_route_text_edit_sort_key(&b.path))
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
            .then_with(|| a.new_target.cmp(&b.new_target))
    });
    text_edits.dedup_by(|left, right| {
        left.path == right.path && left.range == right.range && left.new_target == right.new_target
    });

    Ok(Some(PageRouteRenamePlan {
        old_route: target_page.route.clone(),
        new_route: target.route,
        old_source_file: target_page.source_file.clone(),
        new_source_file: target.source_file,
        text_edits,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PageRouteRenameTarget {
    route: String,
    source_file: String,
}

fn page_route_rename_target(page: &AuthoringPage, new_name: &str) -> Option<PageRouteRenameTarget> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('\n') || new_name.contains('\r') {
        return None;
    }

    if let Some(source_file) = new_name
        .strip_prefix("@/")
        .or_else(|| new_name.ends_with(".md").then_some(new_name))
    {
        let source_file = normalize_relative_path(Utf8Path::new(source_file));
        validate_markdown_source_file(&source_file)?;
        let source_path = SourcePath::new(source_file.clone());
        if source_path.is_section_index() != (page.kind == AuthoringPageKind::Section) {
            return None;
        }
        let route = source_path.to_route().to_string();
        return Some(PageRouteRenameTarget { route, source_file });
    }

    let route = normalize_route(new_name);
    let source_file = source_file_for_page_route(&route, page.kind)?;
    Some(PageRouteRenameTarget { route, source_file })
}

fn page_route_rendered_href_edit(
    project: &AuthoringProject,
    source_page: &AuthoringPage,
    target_page: &AuthoringPage,
    target: &PageRouteRenameTarget,
    href: &RenderedHref,
) -> Option<PageRouteTextEdit> {
    let origin = href.origin.as_ref()?;
    let target_route = rendered_href_target_route(project, source_page, &href.href)?;
    if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
        return None;
    }
    let (base, suffix) = target_base_and_suffix(&href.href);
    if base.is_empty() || !base.starts_with('/') {
        return None;
    }
    let new_target = format!("{}{}", target.route, suffix);
    if new_target == href.href {
        return None;
    }
    Some(PageRouteTextEdit {
        path: origin.path.clone(),
        range: rendered_href_origin_range(project, origin)?,
        new_target,
    })
}

fn rendered_href_origin_range(
    project: &AuthoringProject,
    origin: &RenderedHrefOrigin,
) -> Option<Range> {
    let content = match &origin.path {
        AuthoringInputPath::Source(source_file) => project.source_contents.get(source_file)?,
        AuthoringInputPath::Template(template_file) => {
            project.template_contents.get(template_file)?
        }
        AuthoringInputPath::Sass(_)
        | AuthoringInputPath::Static(_)
        | AuthoringInputPath::Dist(_)
        | AuthoringInputPath::Data(_) => return None,
    };
    (origin.byte_end <= content.len())
        .then(|| byte_range_to_lsp_range(content, origin.byte_start, origin.byte_end))
}

fn page_route_link_edit(
    project: &AuthoringProject,
    page: &AuthoringPage,
    target_page: &AuthoringPage,
    target: &PageRouteRenameTarget,
    context: MarkdownTargetContext,
) -> Option<PageRouteTextEdit> {
    let reference = MarkdownReference {
        kind: context.kind,
        target: context.target.clone(),
        byte_start: context.byte_start,
        byte_end: context.byte_end,
    };
    let target_route = reference_target_route(project, page, &reference)?;
    if !project.routes_refer_to_same_page(&target_route, &target_page.route) {
        return None;
    }

    let (base, suffix) = target_base_and_suffix(&context.target);
    if base.is_empty() {
        return None;
    }

    let effective_source_file = if page.source_file == target_page.source_file {
        target.source_file.as_str()
    } else {
        page.source_file.as_str()
    };
    let effective_link_base_route = if page.source_file == target_page.source_file {
        link_base_route_for_route(&target.route, page.kind)
    } else {
        page.link_base_route.clone()
    };

    let new_base = if base.starts_with("@/") {
        format!("@/{}", target.source_file)
    } else if base.ends_with(".md") {
        relative_source_path_from_source(effective_source_file, &target.source_file)
    } else if base.starts_with('/') {
        target.route.clone()
    } else {
        relative_route_from_base(&effective_link_base_route, &target.route)
    };
    let new_target = format!("{new_base}{suffix}");
    if new_target == context.target {
        return None;
    }

    Some(PageRouteTextEdit {
        path: AuthoringInputPath::Source(if page.source_file == target_page.source_file {
            target.source_file.clone()
        } else {
            page.source_file.clone()
        }),
        range: context.range,
        new_target,
    })
}

fn page_route_text_edit_sort_key(path: &AuthoringInputPath) -> String {
    match path {
        AuthoringInputPath::Source(path) => format!("source:{path}"),
        AuthoringInputPath::Template(path) => format!("template:{path}"),
        AuthoringInputPath::Sass(path) => format!("sass:{path}"),
        AuthoringInputPath::Static(path) => format!("static:{path}"),
        AuthoringInputPath::Dist(path) => format!("dist:{path}"),
        AuthoringInputPath::Data(path) => format!("data:{path}"),
    }
}

fn workspace_edit_for_page_route_rename(
    content_dir: &Utf8Path,
    plan: &PageRouteRenamePlan,
) -> Result<WorkspaceEdit> {
    let old_uri = source_file_uri(content_dir, &plan.old_source_file)?;
    let new_uri = source_file_uri(content_dir, &plan.new_source_file)?;
    let mut operations = vec![DocumentChangeOperation::Op(ResourceOp::Rename(
        RenameFile {
            old_uri: old_uri.clone(),
            new_uri: new_uri.clone(),
            options: Some(RenameFileOptions {
                overwrite: Some(false),
                ignore_if_exists: Some(false),
            }),
            annotation_id: None,
        },
    ))];

    let mut edits_by_path: HashMap<AuthoringInputPath, Vec<TextEdit>> = HashMap::new();
    for edit in &plan.text_edits {
        edits_by_path
            .entry(edit.path.clone())
            .or_default()
            .push(TextEdit::new(edit.range, edit.new_target.clone()));
    }

    let mut paths = edits_by_path.keys().cloned().collect::<Vec<_>>();
    paths.sort_by_key(page_route_text_edit_sort_key);
    for path in paths {
        let uri = page_route_edit_uri(content_dir, &path, plan)?;
        let mut edits = edits_by_path.remove(&path).unwrap_or_default();
        edits.sort_by(|a, b| {
            position_cmp(a.range.start, b.range.start)
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        operations.push(DocumentChangeOperation::Edit(TextDocumentEdit {
            text_document: OptionalVersionedTextDocumentIdentifier { uri, version: None },
            edits: edits.into_iter().map(OneOf::Left).collect(),
        }));
    }

    Ok(WorkspaceEdit {
        changes: None,
        document_changes: Some(DocumentChanges::Operations(operations)),
        change_annotations: None,
    })
}

fn page_route_edit_uri(
    content_dir: &Utf8Path,
    path: &AuthoringInputPath,
    plan: &PageRouteRenamePlan,
) -> Result<Url> {
    match path {
        AuthoringInputPath::Source(source_file) if source_file == &plan.new_source_file => {
            Ok(source_file_uri(content_dir, &plan.new_source_file)?)
        }
        AuthoringInputPath::Source(source_file) => source_file_uri(content_dir, source_file),
        AuthoringInputPath::Template(template_file) => Url::from_file_path(
            content_dir
                .parent()
                .unwrap_or(content_dir)
                .join("templates")
                .join(template_file),
        )
        .map_err(|_| eyre!("could not convert template file to URI: {template_file}")),
        AuthoringInputPath::Sass(path)
        | AuthoringInputPath::Static(path)
        | AuthoringInputPath::Dist(path)
        | AuthoringInputPath::Data(path) => Err(eyre!(
            "page route rename cannot edit unsupported authoring input: {path}"
        )),
    }
}

fn source_file_uri(content_dir: &Utf8Path, source_file: &str) -> Result<Url> {
    Url::from_file_path(content_dir.join(source_file))
        .map_err(|_| eyre!("could not convert source file to URI: {source_file}"))
}

fn target_base_and_suffix(target: &str) -> (&str, &str) {
    let suffix_start = [target.find('#'), target.find('?')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(target.len());
    (&target[..suffix_start], &target[suffix_start..])
}

fn source_file_for_page_route(route: &str, kind: AuthoringPageKind) -> Option<String> {
    match kind {
        AuthoringPageKind::Page => source_file_for_new_route(route),
        AuthoringPageKind::Section => source_file_for_new_section_route(route),
    }
}

fn source_file_for_new_section_route(route: &str) -> Option<String> {
    let route = normalize_route(route);
    let relative = route.strip_prefix('/')?;
    if relative.is_empty() {
        return None;
    }
    validate_route_relative_path(relative)?;
    Some(format!("{relative}/_index.md"))
}

fn validate_markdown_source_file(source_file: &str) -> Option<()> {
    source_file.ends_with(".md").then_some(())?;
    if source_file.starts_with('/') {
        return None;
    }
    validate_route_relative_path(source_file.strip_suffix(".md").unwrap_or(source_file))
}

fn validate_route_relative_path(relative: &str) -> Option<()> {
    if relative.is_empty() {
        return None;
    }
    for segment in relative.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            return None;
        }
    }
    Some(())
}

fn link_base_route_for_route(route: &str, kind: AuthoringPageKind) -> String {
    match kind {
        AuthoringPageKind::Page => route_parent(route),
        AuthoringPageKind::Section => normalize_route(route),
    }
}

fn route_parent(route: &str) -> String {
    let route = normalize_route(route);
    let parts = route_segments(&route);
    if parts.len() <= 1 {
        "/".to_string()
    } else {
        format!("/{}", parts[..parts.len() - 1].join("/"))
    }
}

fn relative_source_path_from_source(source_file: &str, target_source_file: &str) -> String {
    let source_parent = Utf8Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let source_parts = source_parent
        .as_str()
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let target_parts = target_source_file
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let shared = source_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut parts = Vec::new();
    for _ in shared..source_parts.len() {
        parts.push("..".to_string());
    }
    parts.extend(
        target_parts[shared..]
            .iter()
            .map(|part| (*part).to_string()),
    );

    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn reference_target_route(
    project: &AuthoringProject,
    page: &AuthoringPage,
    reference: &MarkdownReference,
) -> Option<String> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return None;
    }

    let (target_without_fragment, _) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        return project.source_to_route.get(source_target).cloned();
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return None;
    }

    Some(route_for_link_target(
        project,
        page,
        target_without_fragment,
    ))
}

fn location_for_markdown_reference(
    content_dir: &Utf8Path,
    source_file: &str,
    content: &str,
    reference: &MarkdownReference,
) -> Option<Location> {
    let uri = Url::from_file_path(content_dir.join(source_file)).ok()?;
    let (line, column) = byte_to_line_column(content, reference.byte_start);
    let (line_end, column_end) = byte_to_line_column(content, reference.byte_end);
    Some(Location {
        uri,
        range: Range {
            start: Position {
                line: line.saturating_sub(1),
                character: column.saturating_sub(1),
            },
            end: Position {
                line: line_end.saturating_sub(1),
                character: column_end.saturating_sub(1),
            },
        },
    })
}

fn location_for_source_path(path: &Utf8Path, fragment: Option<&str>) -> Result<Option<Location>> {
    if !path.exists() {
        return Ok(None);
    }

    let line = match fragment.filter(|fragment| !fragment.is_empty()) {
        Some(fragment) => {
            let content = std::fs::read_to_string(path)?;
            markdown_headings(&content)
                .into_iter()
                .find(|heading| heading.id == fragment)
                .map(|heading| heading.line)
                .unwrap_or(1)
        }
        None => 1,
    };

    Ok(location_for_path(path, line, 1))
}

fn location_for_path(path: &Utf8Path, line: u32, column: u32) -> Option<Location> {
    let uri = Url::from_file_path(path).ok()?;
    let start = Position {
        line: line.saturating_sub(1),
        character: column.saturating_sub(1),
    };
    Some(Location {
        uri,
        range: Range { start, end: start },
    })
}

fn location_for_static_target(
    project: &AuthoringProject,
    source_file: &str,
    target: &str,
) -> Option<Location> {
    let target = strip_query(target);
    if target.starts_with('/') {
        return project
            .static_paths
            .get(target.trim_start_matches('/'))
            .and_then(|path| location_for_path(path, 1, 1));
    }

    let source_parent = Utf8Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let content_relative = source_parent.join(target).to_string();
    project
        .static_paths
        .get(&content_relative)
        .or_else(|| project.static_paths.get(target))
        .and_then(|path| location_for_path(path, 1, 1))
}

fn route_for_link_target(
    project: &AuthoringProject,
    page: &AuthoringPage,
    target_without_fragment: &str,
) -> String {
    if target_without_fragment.is_empty() {
        return page.route.clone();
    }

    if let Some(source_target) =
        source_target_for_relative_markdown_link(page, target_without_fragment)
        && let Some(route) = project.source_to_route.get(&source_target)
    {
        return route.clone();
    }

    if target_without_fragment.starts_with('/') {
        normalize_route(target_without_fragment)
    } else {
        normalize_route(&format!(
            "{}{target_without_fragment}",
            ensure_trailing_slash(&page.link_base_route)
        ))
    }
}

fn source_target_for_relative_markdown_link(page: &AuthoringPage, target: &str) -> Option<String> {
    if !target.ends_with(".md") {
        return None;
    }
    let source_parent = Utf8Path::new(&page.source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    Some(normalize_relative_path(&source_parent.join(target)))
}

fn normalize_relative_path(path: &Utf8Path) -> String {
    let mut parts = Vec::new();
    for segment in path.as_str().split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            segment => parts.push(segment),
        }
    }
    parts.join("/")
}

fn ensure_trailing_slash(route: &str) -> String {
    if route == "/" || route.ends_with('/') {
        route.to_string()
    } else {
        format!("{route}/")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownReferenceKind {
    Link,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownReference {
    kind: MarkdownReferenceKind,
    target: String,
    byte_start: usize,
    byte_end: usize,
}

impl MarkdownReference {
    fn diagnostic(
        &self,
        page: &AuthoringPage,
        content: &str,
        kind: AuthoringDiagnosticKind,
        resolved_route: Option<String>,
        message: String,
    ) -> AuthoringDiagnostic {
        let (line, column) = byte_to_line_column(content, self.byte_start);
        let (line_end, column_end) = byte_to_line_column(content, self.byte_end);
        AuthoringDiagnostic {
            source_file: page.source_file.clone(),
            route: page.route.clone(),
            kind,
            target: self.target.clone(),
            resolved_route,
            message,
            line,
            column,
            line_end,
            column_end,
            byte_start: self.byte_start,
            byte_end: self.byte_end,
        }
    }
}

fn markdown_references(content: &str) -> Vec<MarkdownReference> {
    Parser::new_ext(content, Options::all())
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::Link { dest_url, .. }) => Some(MarkdownReference {
                kind: MarkdownReferenceKind::Link,
                target: dest_url.to_string(),
                byte_start: range.start,
                byte_end: range.end,
            }),
            Event::Start(Tag::Image { dest_url, .. }) => Some(MarkdownReference {
                kind: MarkdownReferenceKind::Image,
                target: dest_url.to_string(),
                byte_start: range.start,
                byte_end: range.end,
            }),
            _ => None,
        })
        .collect()
}

fn reference_at_position(content: &str, position: Position) -> Option<MarkdownReference> {
    let byte_offset = position_to_byte_offset(content, position)?;
    markdown_references(content)
        .into_iter()
        .find(|reference| reference.byte_start <= byte_offset && byte_offset <= reference.byte_end)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownTargetContext {
    kind: MarkdownReferenceKind,
    target: String,
    range: Range,
    byte_start: usize,
    byte_end: usize,
}

fn markdown_target_context_at_position(
    content: &str,
    position: Position,
) -> Option<MarkdownTargetContext> {
    let byte_offset = position_to_byte_offset(content, position)?;
    markdown_target_contexts(content)
        .into_iter()
        .find(|context| {
            let start = lsp_position_to_byte_offset(content, context.range.start);
            let end = lsp_position_to_byte_offset(content, context.range.end);
            match (start, end) {
                (Some(start), Some(end)) => start <= byte_offset && byte_offset <= end,
                _ => false,
            }
        })
}

fn markdown_target_contexts(content: &str) -> Vec<MarkdownTargetContext> {
    let mut contexts = Vec::new();
    let bytes = content.as_bytes();
    let mut cursor = 0;

    while cursor + 1 < bytes.len() {
        if bytes[cursor] != b']' || bytes[cursor + 1] != b'(' {
            cursor += 1;
            continue;
        }

        let target_start = cursor + 2;
        let mut target_end = target_start;
        while target_end < bytes.len() {
            match bytes[target_end] {
                b')' if target_end == target_start || bytes[target_end - 1] != b'\\' => break,
                b'\n' => break,
                _ => target_end += 1,
            }
        }

        if target_end <= bytes.len() {
            let target = &content[target_start..target_end];
            let kind = markdown_reference_kind_before_link_close(content, cursor);
            contexts.push(MarkdownTargetContext {
                kind,
                target: target.to_string(),
                range: byte_range_to_lsp_range(content, target_start, target_end),
                byte_start: target_start,
                byte_end: target_end,
            });
        }

        cursor = target_end.saturating_add(1);
    }

    contexts
}

fn markdown_reference_kind_before_link_close(
    content: &str,
    link_close_byte: usize,
) -> MarkdownReferenceKind {
    let line_start = content[..link_close_byte]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let Some(open_bracket) = content[line_start..link_close_byte].rfind('[') else {
        return MarkdownReferenceKind::Link;
    };
    let open_bracket = line_start + open_bracket;

    if open_bracket > 0 && content.as_bytes()[open_bracket - 1] == b'!' {
        MarkdownReferenceKind::Image
    } else {
        MarkdownReferenceKind::Link
    }
}

fn completion_items_for_markdown_target(
    project: &AuthoringProject,
    page: &AuthoringPage,
    context: &MarkdownTargetContext,
) -> Vec<CompletionItem> {
    let target = context.target.as_str();
    if is_special_target(target) {
        return Vec::new();
    }

    if target.starts_with('#') || target.contains('#') {
        return heading_completion_items(project, page, context);
    }

    if target.starts_with('@') {
        return source_completion_items(project, context);
    }

    if context.kind == MarkdownReferenceKind::Image {
        return static_completion_items(project, context);
    }

    let mut items = route_completion_items(project, page, context);
    if target.starts_with('/') || is_likely_static_file(target) {
        items.extend(static_completion_items(project, context));
    }
    items
}

fn route_completion_items(
    project: &AuthoringProject,
    page: &AuthoringPage,
    context: &MarkdownTargetContext,
) -> Vec<CompletionItem> {
    project
        .pages
        .iter()
        .map(|candidate| {
            let insert_text = route_completion_text(page, &context.target, &candidate.route);
            completion_item(
                insert_text,
                CompletionItemKind::REFERENCE,
                format!("{} ({})", candidate.title, candidate.source_file),
                context.range,
            )
        })
        .collect()
}

fn source_completion_items(
    project: &AuthoringProject,
    context: &MarkdownTargetContext,
) -> Vec<CompletionItem> {
    let mut sources = project.source_to_route.keys().collect::<Vec<_>>();
    sources.sort();

    sources
        .into_iter()
        .map(|source_file| {
            let route = project
                .source_to_route
                .get(source_file)
                .map(String::as_str)
                .unwrap_or("");
            completion_item(
                format!("@/{source_file}"),
                CompletionItemKind::FILE,
                format!("Dodeca source route {route}"),
                context.range,
            )
        })
        .collect()
}

fn static_completion_items(
    project: &AuthoringProject,
    context: &MarkdownTargetContext,
) -> Vec<CompletionItem> {
    let mut paths = project.static_paths.keys().collect::<Vec<_>>();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            completion_item(
                format!("/{path}"),
                CompletionItemKind::FILE,
                "Dodeca static asset".to_string(),
                context.range,
            )
        })
        .collect()
}

fn heading_completion_items(
    project: &AuthoringProject,
    page: &AuthoringPage,
    context: &MarkdownTargetContext,
) -> Vec<CompletionItem> {
    let (target_without_fragment, _) = split_fragment(&context.target);
    let target_route = if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        project.source_to_route.get(source_target).cloned()
    } else {
        Some(route_for_link_target(
            project,
            page,
            target_without_fragment,
        ))
    };
    let Some(target_route) = target_route else {
        return Vec::new();
    };
    let Some(target_page) = project.page_for_route(&target_route) else {
        return Vec::new();
    };

    let prefix = if context.target.starts_with('#') {
        String::new()
    } else {
        target_without_fragment.to_string()
    };

    target_page
        .heading_ids
        .iter()
        .map(|heading_id| {
            let insert_text = format!("{prefix}#{heading_id}");
            completion_item(
                insert_text,
                CompletionItemKind::REFERENCE,
                format!("Heading on {}", target_page.route),
                context.range,
            )
        })
        .collect()
}

fn completion_item(
    insert_text: String,
    kind: CompletionItemKind,
    detail: String,
    range: Range,
) -> CompletionItem {
    CompletionItem {
        label: insert_text.clone(),
        kind: Some(kind),
        detail: Some(detail),
        filter_text: Some(insert_text.clone()),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: insert_text,
        })),
        ..CompletionItem::default()
    }
}

fn template_completion_item(
    insert_text: String,
    kind: CompletionItemKind,
    info: TemplateItemInfo,
    range: Range,
) -> CompletionItem {
    CompletionItem {
        label: insert_text.clone(),
        kind: Some(kind),
        detail: Some(info.detail.to_string()),
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: info.documentation.to_string(),
        })),
        filter_text: Some(insert_text.clone()),
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: insert_text,
        })),
        ..CompletionItem::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplateCompletionKind {
    Root,
    Field(Vec<String>),
    Filter,
    Test,
    Macro(String),
}

#[derive(Debug, Clone)]
struct TemplateCompletionContext {
    kind: TemplateCompletionKind,
    range: Range,
}

const TEMPLATE_ROOT_VALUES: &[&str] =
    &["config", "page", "section", "current_path", "root", "data"];
const TEMPLATE_CONFIG_FIELDS: &[&str] = &["title", "description", "base_url"];
const TEMPLATE_PAGE_FIELDS: &[&str] = &[
    "title",
    "content",
    "permalink",
    "path",
    "weight",
    "toc",
    "ancestors",
    "last_updated",
    "description",
    "extra",
];
const TEMPLATE_SECTION_FIELDS: &[&str] = &[
    "title",
    "content",
    "permalink",
    "path",
    "weight",
    "last_updated",
    "ancestors",
    "pages",
    "subsections",
    "toc",
    "extra",
];

#[derive(Debug, Clone, Copy)]
struct TemplateItemInfo {
    detail: &'static str,
    documentation: &'static str,
}

fn template_root_info(name: &str) -> TemplateItemInfo {
    match name {
        "config" => TemplateItemInfo {
            detail: "Dodeca site configuration",
            documentation: "Global site metadata such as `title`, `description`, `base_url`.",
        },
        "page" => TemplateItemInfo {
            detail: "Current page",
            documentation: "The page currently being rendered. On section templates this is `null`.",
        },
        "section" => TemplateItemInfo {
            detail: "Current section",
            documentation: "The current section, including its rendered content, pages, subsections, headings, and metadata.",
        },
        "current_path" => TemplateItemInfo {
            detail: "Current route",
            documentation: "Route path for the page or section currently being rendered.",
        },
        "root" => TemplateItemInfo {
            detail: "Root section",
            documentation: "The root section object for site-wide navigation and metadata.",
        },
        "data" => TemplateItemInfo {
            detail: "Data registry",
            documentation: "Lazy access to parsed files from the Dodeca `data/` directory.",
        },
        _ => TemplateItemInfo {
            detail: "Dodeca template context",
            documentation: "A value supplied by Dodeca to Gingembre templates.",
        },
    }
}

fn template_field_info(path: &[String], name: &str) -> TemplateItemInfo {
    let root = path.first().map(String::as_str).unwrap_or("");
    match (root, name) {
        (_, "title") => TemplateItemInfo {
            detail: "Dodeca title field",
            documentation: "Human-readable title from frontmatter or Dodeca's source-path fallback.",
        },
        (_, "content") => TemplateItemInfo {
            detail: "Rendered Markdown body",
            documentation: "HTML body produced from the source Markdown before the surrounding template is applied.",
        },
        (_, "permalink") => TemplateItemInfo {
            detail: "Absolute permalink",
            documentation: "Absolute URL for this page or section, built from the site `base_url`.",
        },
        (_, "path") => TemplateItemInfo {
            detail: "Route path",
            documentation: "Site-relative route for this page or section, such as `/guide/intro`.",
        },
        (_, "weight") => TemplateItemInfo {
            detail: "Ordering weight",
            documentation: "Numeric frontmatter weight used when Dodeca sorts pages and sections.",
        },
        (_, "toc") => TemplateItemInfo {
            detail: "Heading table of contents",
            documentation: "Structured heading list extracted from the rendered Markdown source.",
        },
        (_, "ancestors") => TemplateItemInfo {
            detail: "Ancestor sections",
            documentation: "Parent section chain for breadcrumb and navigation templates.",
        },
        (_, "last_updated") => TemplateItemInfo {
            detail: "Last source update timestamp",
            documentation: "Filesystem timestamp captured from the source file when Dodeca loaded it.",
        },
        (_, "description") => TemplateItemInfo {
            detail: "Page or site description",
            documentation: "Optional description from frontmatter or site configuration.",
        },
        (_, "extra") => TemplateItemInfo {
            detail: "Frontmatter extra table",
            documentation: "Custom `[extra]` frontmatter values preserved for template use.",
        },
        ("section" | "root", "pages") => TemplateItemInfo {
            detail: "Section pages",
            documentation: "Pages whose nearest parent section is this section, sorted by weight.",
        },
        ("section" | "root", "subsections") => TemplateItemInfo {
            detail: "Child sections",
            documentation: "Immediate child sections below this section, sorted by weight.",
        },
        _ => TemplateItemInfo {
            detail: "Dodeca template field",
            documentation: "Field supplied by Dodeca in the Gingembre template context.",
        },
    }
}

fn template_function_info(name: &str) -> TemplateItemInfo {
    match name {
        "get_url" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Builds a site URL for a static path and lets Dodeca rewrite/cache-bust it during rendering.",
        },
        "get_section" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Loads a section object by route for navigation and cross-section templates.",
        },
        "now" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Returns the current build-time timestamp.",
        },
        "throw" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Aborts template rendering with an explicit error message.",
        },
        "build" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Accesses named build-step output exposed to templates.",
        },
        "read" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Reads a project file for template-driven authoring workflows.",
        },
        "highlight" => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Highlights source code using Dodeca's syntax-highlighting pipeline.",
        },
        _ => TemplateItemInfo {
            detail: "Dodeca template function",
            documentation: "Function supplied by Dodeca to Gingembre templates.",
        },
    }
}

fn template_filter_info(name: &str) -> TemplateItemInfo {
    builtin_filter(name)
        .map(template_builtin_info)
        .unwrap_or(TemplateItemInfo {
            detail: "Gingembre filter",
            documentation: "Built-in Gingembre filter.",
        })
}

fn template_test_info(name: &str) -> TemplateItemInfo {
    builtin_test(name)
        .map(template_builtin_info)
        .unwrap_or(TemplateItemInfo {
            detail: "Gingembre test",
            documentation: "Built-in Gingembre test.",
        })
}

fn template_builtin_info(info: &BuiltinItemInfo) -> TemplateItemInfo {
    TemplateItemInfo {
        detail: info.detail,
        documentation: info.documentation,
    }
}

fn template_symbol_info(symbol: &TemplateSymbol) -> TemplateItemInfo {
    match symbol.kind {
        TemplateSymbolKind::SetBinding => TemplateItemInfo {
            detail: "Gingembre local variable",
            documentation: "Variable introduced by a `{% set %}` statement in this template scope.",
        },
        TemplateSymbolKind::LoopBinding => TemplateItemInfo {
            detail: "Gingembre loop variable",
            documentation: "Variable bound by a `{% for %}` loop and visible inside the loop body.",
        },
        TemplateSymbolKind::MacroParam => TemplateItemInfo {
            detail: "Gingembre macro parameter",
            documentation: "Parameter available inside this macro body.",
        },
        TemplateSymbolKind::ImportAlias => TemplateItemInfo {
            detail: "Gingembre import alias",
            documentation: "Namespace introduced by `{% import \"...\" as name %}`.",
        },
        TemplateSymbolKind::Macro => TemplateItemInfo {
            detail: "Gingembre macro",
            documentation: "Macro declared in this template.",
        },
        TemplateSymbolKind::ContextRoot => template_root_info(&symbol.name),
        TemplateSymbolKind::Function => template_function_info(&symbol.name),
    }
}

fn completion_kind_for_template_symbol(kind: TemplateSymbolKind) -> CompletionItemKind {
    match kind {
        TemplateSymbolKind::Function => CompletionItemKind::FUNCTION,
        TemplateSymbolKind::ImportAlias => CompletionItemKind::MODULE,
        TemplateSymbolKind::Macro => CompletionItemKind::FUNCTION,
        TemplateSymbolKind::MacroParam | TemplateSymbolKind::LoopBinding => {
            CompletionItemKind::VARIABLE
        }
        TemplateSymbolKind::ContextRoot | TemplateSymbolKind::SetBinding => {
            CompletionItemKind::VARIABLE
        }
    }
}

fn template_completion_items(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Vec<CompletionItem> {
    let Some(context) = template_completion_context(content, position) else {
        return Vec::new();
    };

    match &context.kind {
        TemplateCompletionKind::Root => {
            template_root_completion_items(project, template_file, content, &context)
        }
        TemplateCompletionKind::Field(path) => {
            let offset = position_to_byte_offset(content, position);
            template_field_completion_items(project, template_file, path, context.range, offset)
        }
        TemplateCompletionKind::Filter => BUILTIN_FILTERS
            .iter()
            .map(|info| {
                template_completion_item(
                    info.name.to_string(),
                    CompletionItemKind::FUNCTION,
                    template_builtin_info(info),
                    context.range,
                )
            })
            .collect(),
        TemplateCompletionKind::Test => BUILTIN_TESTS
            .iter()
            .map(|info| {
                template_completion_item(
                    info.name.to_string(),
                    CompletionItemKind::FUNCTION,
                    template_builtin_info(info),
                    context.range,
                )
            })
            .collect(),
        TemplateCompletionKind::Macro(namespace) => template_macro_completion_items(
            project,
            template_file,
            content,
            namespace,
            context.range,
        ),
    }
}

fn template_root_completion_items(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    context: &TemplateCompletionContext,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for name in TEMPLATE_ROOT_VALUES {
        items.push(template_completion_item(
            (*name).to_string(),
            CompletionItemKind::VARIABLE,
            template_root_info(name),
            context.range,
        ));
    }
    for name in TEMPLATE_FUNCTION_NAMES {
        items.push(template_completion_item(
            (*name).to_string(),
            CompletionItemKind::FUNCTION,
            template_function_info(name),
            context.range,
        ));
    }
    if let Some(index) = project.template_semantics.get(template_file)
        && let Some(offset) = lsp_position_to_byte_offset(content, context.range.start)
    {
        for symbol in index.visible_symbols_at_offset(offset) {
            if matches!(
                symbol.kind,
                TemplateSymbolKind::ContextRoot | TemplateSymbolKind::Function
            ) {
                continue;
            }
            items.push(template_completion_item(
                symbol.name.clone(),
                completion_kind_for_template_symbol(symbol.kind),
                template_symbol_info(symbol),
                context.range,
            ));
        }
    }

    let import_aliases = TemplateParser::new(template_file, content)
        .parse()
        .map(|template| template_import_aliases(project, &template.body))
        .unwrap_or_default();
    for alias in import_aliases.keys() {
        items.push(template_completion_item(
            alias.clone(),
            CompletionItemKind::MODULE,
            TemplateItemInfo {
                detail: "Dodeca import alias",
                documentation: "Namespace for macros imported from another Gingembre template.",
            },
            context.range,
        ));
    }

    if let Ok(template) = TemplateParser::new(template_file, content).parse() {
        if top_level_macro_names(&template.body).next().is_some() {
            items.push(template_completion_item(
                "self".to_string(),
                CompletionItemKind::MODULE,
                TemplateItemInfo {
                    detail: "Dodeca macro namespace",
                    documentation: "`self` refers to macros declared in the current template.",
                },
                context.range,
            ));
        }
    }

    items
}

fn template_field_completion_items(
    project: &AuthoringProject,
    template_file: &str,
    path: &[String],
    range: Range,
    offset: Option<usize>,
) -> Vec<CompletionItem> {
    let resolved_path = template_resolved_field_path(project, template_file, path, offset)
        .unwrap_or_else(|| path.to_vec());
    let names = match resolved_path.as_slice() {
        [root] if root == "config" => TEMPLATE_CONFIG_FIELDS,
        [root] if root == "page" => TEMPLATE_PAGE_FIELDS,
        [root] if root == "section" || root == "root" => TEMPLATE_SECTION_FIELDS,
        [root] if root == "data" => {
            return project
                .data_keys
                .iter()
                .map(|name| {
                    template_completion_item(
                        name.clone(),
                        CompletionItemKind::FIELD,
                        TemplateItemInfo {
                            detail: "Dodeca data file",
                            documentation: "Top-level key derived from a file in the Dodeca `data/` directory.",
                        },
                        range,
                    )
                })
                .collect();
        }
        _ => return Vec::new(),
    };

    names
        .iter()
        .map(|name| {
            template_completion_item(
                (*name).to_string(),
                CompletionItemKind::FIELD,
                template_field_info(&resolved_path, name),
                range,
            )
        })
        .collect()
}

fn template_resolved_field_path(
    project: &AuthoringProject,
    template_file: &str,
    path: &[String],
    offset: Option<usize>,
) -> Option<Vec<String>> {
    let offset = offset?;
    let index = project.template_semantics.get(template_file)?;
    resolve_template_expression_path(index, path, offset, 0)
}

fn resolve_template_expression_path(
    index: &TemplateSemanticIndex,
    path: &[String],
    offset: usize,
    depth: usize,
) -> Option<Vec<String>> {
    if depth > 8 || path.is_empty() {
        return None;
    }
    let root = &path[0];
    let suffix = &path[1..];
    let symbol = index.visible_symbol_named_at_offset(root, offset)?;
    let mut resolved = resolve_template_symbol_path(index, symbol, offset, depth + 1)?;
    resolved.extend(suffix.iter().cloned());
    Some(resolved)
}

fn resolve_template_symbol_path(
    index: &TemplateSemanticIndex,
    symbol: &TemplateSymbol,
    offset: usize,
    depth: usize,
) -> Option<Vec<String>> {
    if depth > 8 {
        return None;
    }
    match &symbol.origin {
        Some(TemplateSymbolOrigin::ContextRoot) => Some(vec![symbol.name.clone()]),
        Some(TemplateSymbolOrigin::ExpressionPath(path)) => {
            resolve_template_expression_path(index, path, offset, depth + 1)
                .or_else(|| Some(path.clone()))
        }
        Some(TemplateSymbolOrigin::IterationItem(path)) => {
            let iter_path = resolve_template_expression_path(index, path, offset, depth + 1)
                .unwrap_or_else(|| path.clone());
            template_iteration_item_path(&iter_path)
        }
        _ => None,
    }
}

fn template_iteration_item_path(iter_path: &[String]) -> Option<Vec<String>> {
    match iter_path {
        [root, field] if (root == "section" || root == "root") && field == "pages" => {
            Some(vec!["page".to_string()])
        }
        [root, field] if (root == "section" || root == "root") && field == "subsections" => {
            Some(vec!["section".to_string()])
        }
        _ => None,
    }
}

fn template_macro_completion_items(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    namespace: &str,
    range: Range,
) -> Vec<CompletionItem> {
    let target_file = if namespace == "self" {
        Some(template_file.to_string())
    } else {
        TemplateParser::new(template_file, content)
            .parse()
            .ok()
            .and_then(|template| {
                template_import_aliases(project, &template.body)
                    .get(namespace)
                    .cloned()
            })
    };
    let Some(target_file) = target_file else {
        return Vec::new();
    };
    let Some(target_content) = template_content(project, &target_file, template_file, content)
    else {
        return Vec::new();
    };
    let Ok(template) = TemplateParser::new(target_file, target_content).parse() else {
        return Vec::new();
    };

    top_level_macro_names(&template.body)
        .map(|name| {
            template_completion_item(
                name,
                CompletionItemKind::FUNCTION,
                TemplateItemInfo {
                    detail: "Dodeca template macro",
                    documentation: "Macro callable from the selected Gingembre macro namespace.",
                },
                range,
            )
        })
        .collect()
}

fn top_level_macro_names(nodes: &[Node]) -> impl Iterator<Item = String> + '_ {
    nodes.iter().filter_map(|node| match node {
        Node::Macro(node) => Some(node.name.name.clone()),
        _ => None,
    })
}

fn template_completion_context(
    content: &str,
    position: Position,
) -> Option<TemplateCompletionContext> {
    let offset = position_to_byte_offset(content, position)?;
    let (line_start, _) = line_bounds_at_offset(content, offset);
    let replace_start = scan_template_ident_start(content, line_start, offset);
    let replace_end = scan_template_ident_end(content, offset);
    let range = byte_range_to_lsp_range(content, replace_start, replace_end);
    let before_replace = &content[line_start..replace_start];
    let tag_start = before_replace
        .rfind("{{")
        .or_else(|| before_replace.rfind("{%"))?;
    let tag_prefix = &before_replace[tag_start..];

    if let Some(namespace) = template_macro_completion_namespace(tag_prefix) {
        return Some(TemplateCompletionContext {
            kind: TemplateCompletionKind::Macro(namespace),
            range,
        });
    }

    if template_test_completion_requested(tag_prefix) {
        return Some(TemplateCompletionContext {
            kind: TemplateCompletionKind::Test,
            range,
        });
    }

    if template_filter_completion_requested(tag_prefix) {
        return Some(TemplateCompletionContext {
            kind: TemplateCompletionKind::Filter,
            range,
        });
    }

    if let Some(path) = template_field_completion_path(content, line_start, replace_start) {
        return Some(TemplateCompletionContext {
            kind: TemplateCompletionKind::Field(path),
            range,
        });
    }

    Some(TemplateCompletionContext {
        kind: TemplateCompletionKind::Root,
        range,
    })
}

fn template_macro_completion_namespace(tag_prefix: &str) -> Option<String> {
    let before_colons = tag_prefix.strip_suffix("::")?;
    let namespace_end = before_colons.len();
    let namespace_start = before_colons[..namespace_end]
        .rfind(|c: char| !is_template_ident_char(c))
        .map(|index| index + 1)
        .unwrap_or(0);
    (namespace_start < namespace_end)
        .then(|| before_colons[namespace_start..namespace_end].to_string())
}

fn template_test_completion_requested(tag_prefix: &str) -> bool {
    let Some(test_start) = tag_prefix.rfind(" is ") else {
        return tag_prefix.contains(" is not ");
    };
    tag_prefix
        .rfind('|')
        .map(|pipe| test_start > pipe)
        .unwrap_or(true)
}

fn template_filter_completion_requested(tag_prefix: &str) -> bool {
    let Some(pipe) = tag_prefix.rfind('|') else {
        return false;
    };
    tag_prefix
        .rfind(" is ")
        .map(|test_start| pipe > test_start)
        .unwrap_or(true)
}

fn template_field_completion_path(
    content: &str,
    line_start: usize,
    replace_start: usize,
) -> Option<Vec<String>> {
    if replace_start == line_start {
        return None;
    }
    let mut start = replace_start;
    while start > line_start {
        let previous = content[..start].chars().next_back()?;
        if is_template_ident_char(previous) || previous == '.' {
            start -= previous.len_utf8();
        } else {
            break;
        }
    }
    let raw_expr = &content[start..replace_start];
    if !raw_expr.contains('.') {
        return None;
    }
    let expr = raw_expr.trim_end_matches('.');
    let path = expr
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!path.is_empty()).then_some(path)
}

fn scan_template_ident_start(content: &str, lower_bound: usize, offset: usize) -> usize {
    let mut cursor = offset;
    while cursor > lower_bound {
        let Some(ch) = content[..cursor].chars().next_back() else {
            break;
        };
        if !is_template_ident_char(ch) {
            break;
        }
        cursor -= ch.len_utf8();
    }
    cursor
}

fn scan_template_ident_end(content: &str, offset: usize) -> usize {
    let mut cursor = offset;
    while cursor < content.len() {
        let Some(ch) = content[cursor..].chars().next() else {
            break;
        };
        if !is_template_ident_char(ch) {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn is_template_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

fn route_completion_text(page: &AuthoringPage, current_target: &str, target_route: &str) -> String {
    if current_target.starts_with('/') {
        target_route.to_string()
    } else {
        relative_route_from_base(&page.link_base_route, target_route)
    }
}

fn relative_route_from_base(base_route: &str, target_route: &str) -> String {
    if target_route == "/" {
        return "/".to_string();
    }

    let base_parts = route_segments(base_route);
    let target_parts = route_segments(target_route);
    let shared = base_parts
        .iter()
        .zip(target_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();

    let mut parts = Vec::new();
    for _ in shared..base_parts.len() {
        parts.push("..".to_string());
    }
    parts.extend(
        target_parts[shared..]
            .iter()
            .map(|part| (*part).to_string()),
    );

    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn route_segments(route: &str) -> Vec<&str> {
    route
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn byte_range_to_lsp_range(content: &str, byte_start: usize, byte_end: usize) -> Range {
    let (line, column) = byte_to_line_column(content, byte_start);
    let (line_end, column_end) = byte_to_line_column(content, byte_end);
    Range {
        start: Position {
            line: line.saturating_sub(1),
            character: column.saturating_sub(1),
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: column_end.saturating_sub(1),
        },
    }
}

fn lsp_position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
    position_to_byte_offset(content, position)
}

fn lsp_range_to_byte_range(content: &str, range: Range) -> Option<(usize, usize)> {
    let start = lsp_position_to_byte_offset(content, range.start)?;
    let end = lsp_position_to_byte_offset(content, range.end)?;
    (start <= end).then_some((start, end))
}

fn heading_id_at_position(
    page: &AuthoringPage,
    content: &str,
    position: Position,
) -> Option<String> {
    let line = position.line + 1;
    markdown_headings(content)
        .into_iter()
        .find(|heading| heading.line == line && page.heading_ids.contains(&heading.id))
        .map(|heading| heading.id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HeadingRenameTarget {
    heading_id: String,
    title: String,
    line: u32,
    title_range: Range,
}

fn heading_rename_target_at_position(
    page: &AuthoringPage,
    content: &str,
    position: Position,
) -> Option<HeadingRenameTarget> {
    markdown_headings(content)
        .into_iter()
        .find(|heading| {
            range_contains_position(&heading.title_range, position)
                && page
                    .headings
                    .iter()
                    .any(|model_heading| model_heading.id == heading.id)
        })
        .map(|heading| HeadingRenameTarget {
            heading_id: heading.id,
            title: heading.title,
            line: heading.line,
            title_range: heading.title_range,
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownHeading {
    id: String,
    title: String,
    level: u8,
    line: u32,
    title_range: Range,
}

fn markdown_headings(content: &str) -> Vec<MarkdownHeading> {
    let mut headings = Vec::new();
    let mut heading_stack: Vec<(u8, String)> = Vec::new();
    let mut current_heading: Option<(u8, usize, String)> = None;

    for (event, range) in Parser::new_ext(content, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current_heading = Some((level as u8, range.start, String::new()));
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, heading_text)) = &mut current_heading {
                    heading_text.push_str(&text);
                }
            }
            Event::End(TagEnd::Heading(level)) => {
                if let Some((_, byte_start, heading_text)) = current_heading.take() {
                    let current_level = level as u8;
                    let slug = marq::slugify(&heading_text);
                    while heading_stack
                        .last()
                        .is_some_and(|(level, _)| *level >= current_level)
                    {
                        heading_stack.pop();
                    }

                    let id = if heading_stack.is_empty() {
                        slug.clone()
                    } else {
                        let mut id = String::new();
                        for (_, parent_slug) in &heading_stack {
                            id.push_str(parent_slug);
                            id.push_str("--");
                        }
                        id.push_str(&slug);
                        id
                    };

                    heading_stack.push((current_level, slug));
                    let (line, _) = byte_to_line_column(content, byte_start);
                    let title_range = heading_title_range(content, byte_start)
                        .unwrap_or_else(|| one_line_range(line.saturating_sub(1)));
                    headings.push(MarkdownHeading {
                        id,
                        title: heading_text,
                        level: current_level,
                        line,
                        title_range,
                    });
                }
            }
            _ => {}
        }
    }

    headings
}

fn heading_title_range(content: &str, heading_byte_start: usize) -> Option<Range> {
    let line_start = content[..heading_byte_start]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let line_end = content[heading_byte_start..]
        .find('\n')
        .map(|idx| heading_byte_start + idx)
        .unwrap_or(content.len());
    let line = &content[line_start..line_end];
    let bytes = line.as_bytes();
    let mut offset = 0;

    while offset < bytes.len() && offset < 3 && bytes[offset] == b' ' {
        offset += 1;
    }

    let marker_start = offset;
    while offset < bytes.len() && bytes[offset] == b'#' {
        offset += 1;
    }
    if offset == marker_start {
        let title_start = marker_start;
        let mut title_end = bytes.len();
        while title_end > title_start && matches!(bytes[title_end - 1], b' ' | b'\t') {
            title_end -= 1;
        }
        return Some(byte_range_to_lsp_range(
            content,
            line_start + title_start,
            line_start + title_end,
        ));
    }

    while offset < bytes.len() && matches!(bytes[offset], b' ' | b'\t') {
        offset += 1;
    }

    let title_start = offset;
    let mut title_end = bytes.len();
    while title_end > title_start && matches!(bytes[title_end - 1], b' ' | b'\t') {
        title_end -= 1;
    }

    let closing_start = title_end
        - bytes[..title_end]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'#')
            .count();
    if closing_start < title_end
        && closing_start > title_start
        && matches!(bytes[closing_start - 1], b' ' | b'\t')
    {
        title_end = closing_start - 1;
        while title_end > title_start && matches!(bytes[title_end - 1], b' ' | b'\t') {
            title_end -= 1;
        }
    }

    Some(byte_range_to_lsp_range(
        content,
        line_start + title_start,
        line_start + title_end,
    ))
}

fn frontmatter_lsp_range(content: &str) -> Option<Range> {
    content.strip_prefix("+++\n")?;
    let end = content[4..].find("\n+++")? + 4 + "\n+++".len();
    let (line_end, column_end) = byte_to_line_column(content, end);

    Some(Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: Position {
            line: line_end.saturating_sub(1),
            character: column_end.saturating_sub(1),
        },
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterFieldKind {
    String,
    Integer,
    Table,
}

impl FrontmatterFieldKind {
    fn description(self) -> &'static str {
        match self {
            FrontmatterFieldKind::String => "a string",
            FrontmatterFieldKind::Integer => "an integer",
            FrontmatterFieldKind::Table => "a table",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FrontmatterFieldSpec {
    name: &'static str,
    kind: FrontmatterFieldKind,
}

#[derive(Debug, Clone)]
struct FrontmatterEntry {
    key: String,
    key_start: usize,
    key_end: usize,
    value: String,
    value_start: usize,
    value_end: usize,
    table: Option<String>,
}

#[derive(Debug, Clone)]
struct FrontmatterCompletionContext {
    replace_range: Range,
    present_fields: HashSet<String>,
    current_table: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterDocumentKind {
    Template,
    StaticAsset,
    DataFile,
}

impl FrontmatterDocumentKind {
    fn label(self) -> &'static str {
        match self {
            FrontmatterDocumentKind::Template => "template",
            FrontmatterDocumentKind::StaticAsset => "static asset",
            FrontmatterDocumentKind::DataFile => "data file",
        }
    }
}

#[derive(Debug, Clone)]
struct FrontmatterDocumentTarget {
    kind: FrontmatterDocumentKind,
    path: String,
    target_path: Utf8PathBuf,
    source_range: Range,
}

impl FrontmatterDocumentTarget {
    fn target_uri(&self) -> Result<Url> {
        Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert {} path to URI: {}",
                self.kind.label(),
                self.target_path
            )
        })
    }

    fn tooltip(&self) -> String {
        format!("Open Dodeca {} `{}`", self.kind.label(), self.path)
    }

    fn hover_markdown(&self) -> String {
        format!(
            "**Dodeca {}**\n\n`{}`\n\nSource: `{}`",
            self.kind.label(),
            self.path,
            self.target_path
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateDocumentKind {
    Extends,
    Include,
    Import,
}

impl TemplateDocumentKind {
    fn label(self) -> &'static str {
        match self {
            TemplateDocumentKind::Extends => "extends",
            TemplateDocumentKind::Include => "include",
            TemplateDocumentKind::Import => "import",
        }
    }
}

#[derive(Debug, Clone)]
struct TemplateDocumentTarget {
    kind: TemplateDocumentKind,
    path: String,
    target_path: Utf8PathBuf,
    source_range: Range,
}

impl TemplateDocumentTarget {
    fn target_uri(&self) -> Result<Url> {
        Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert template path to URI: {}",
                self.target_path
            )
        })
    }

    fn tooltip(&self) -> String {
        format!("Open Dodeca template {} `{}`", self.kind.label(), self.path)
    }

    fn hover_markdown(&self) -> String {
        format!(
            "**Dodeca template {}**\n\n`{}`\n\nSource: `{}`",
            self.kind.label(),
            self.path,
            self.target_path
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateDefinitionKind {
    Block,
    Macro,
    Filter,
    Test,
}

impl TemplateDefinitionKind {
    fn label(self) -> &'static str {
        match self {
            TemplateDefinitionKind::Block => "block",
            TemplateDefinitionKind::Macro => "macro",
            TemplateDefinitionKind::Filter => "filter",
            TemplateDefinitionKind::Test => "test",
        }
    }
}

#[derive(Debug, Clone)]
struct TemplateDefinitionTarget {
    kind: TemplateDefinitionKind,
    name: String,
    source_range: Range,
    target_path: Utf8PathBuf,
    target_range: Range,
}

impl TemplateDefinitionTarget {
    fn location(&self) -> Result<Location> {
        Ok(Location {
            uri: Url::from_file_path(self.target_path.as_std_path()).map_err(|_| {
                eyre!(
                    "could not convert template definition path to URI: {}",
                    self.target_path
                )
            })?,
            range: self.target_range,
        })
    }

    fn hover_markdown(&self) -> String {
        let info = match self.kind {
            TemplateDefinitionKind::Filter => builtin_filter(&self.name).map(template_builtin_info),
            TemplateDefinitionKind::Test => builtin_test(&self.name).map(template_builtin_info),
            TemplateDefinitionKind::Block | TemplateDefinitionKind::Macro => None,
        };
        if let Some(info) = info {
            return format!(
                "**{}**\n\n`{}`\n\n{}\n\nDefinition: `{}`",
                info.detail, self.name, info.documentation, self.target_path
            );
        }
        format!(
            "**Dodeca template {}**\n\n`{}`\n\nDefinition: `{}`",
            self.kind.label(),
            self.name,
            self.target_path
        )
    }
}

fn frontmatter_field_specs() -> Vec<FrontmatterFieldSpec> {
    let fields = match <Frontmatter as Facet>::SHAPE.ty {
        Type::User(UserType::Struct(struct_type)) => struct_type.fields,
        _ => return Vec::new(),
    };

    fields
        .iter()
        .filter_map(|field| {
            let name = field.rename.unwrap_or(field.name);
            frontmatter_field_kind(name, field.shape.get())
                .map(|kind| FrontmatterFieldSpec { name, kind })
        })
        .collect()
}

fn frontmatter_field_kind(
    name: &'static str,
    shape: &'static facet::Shape,
) -> Option<FrontmatterFieldKind> {
    if name == "extra" {
        return Some(FrontmatterFieldKind::Table);
    }

    let shape = if shape.type_identifier == "Option" {
        shape.inner.unwrap_or(shape)
    } else {
        shape
    };

    if shape.type_identifier == "String" {
        return Some(FrontmatterFieldKind::String);
    }

    match shape.ty {
        Type::Primitive(PrimitiveType::Textual(_)) => Some(FrontmatterFieldKind::String),
        Type::Primitive(PrimitiveType::Numeric(NumericType::Integer { .. })) => {
            Some(FrontmatterFieldKind::Integer)
        }
        _ => None,
    }
}

fn frontmatter_entries(content: &str) -> Vec<FrontmatterEntry> {
    let Some(block) = frontmatter_content_byte_range(content) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    let mut table = None;
    let mut line_start = block.start;

    while line_start < block.end {
        let line_end = content[line_start..block.end]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(block.end);
        let line = &content[line_start..line_end];
        let trimmed = line.trim();

        if let Some(table_name) = frontmatter_table_name(trimmed) {
            table = Some(table_name);
        } else if !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && let Some(entry) =
                frontmatter_entry_for_line(content, line_start, line, table.clone())
        {
            entries.push(entry);
        }

        line_start = line_end.saturating_add(1);
    }

    entries
}

fn frontmatter_entry_for_line(
    content: &str,
    line_start: usize,
    line: &str,
    table: Option<String>,
) -> Option<FrontmatterEntry> {
    let key_start_in_line = line.find(|c: char| !c.is_whitespace())?;
    let key_tail = &line[key_start_in_line..];
    let key_len = key_tail
        .find(|c: char| !is_frontmatter_key_char(c))
        .unwrap_or(key_tail.len());
    if key_len == 0 {
        return None;
    }

    let after_key = key_start_in_line + key_len;
    let equals_offset = line[after_key..].find('=')? + after_key;
    if !line[after_key..equals_offset]
        .chars()
        .all(char::is_whitespace)
    {
        return None;
    }

    let key_start = line_start + key_start_in_line;
    let key_end = key_start + key_len;
    let value_start_in_line =
        equals_offset + 1 + leading_whitespace_len(&line[equals_offset + 1..]);
    let value_end_in_line = line_comment_start(&line[value_start_in_line..])
        .map(|offset| value_start_in_line + offset)
        .unwrap_or(line.len());
    let value_end_in_line = value_end_in_line - trailing_whitespace_len(&line[..value_end_in_line]);
    let value_start = line_start + value_start_in_line;
    let value_end = line_start + value_end_in_line.max(value_start_in_line);

    Some(FrontmatterEntry {
        key: content[key_start..key_end].to_string(),
        key_start,
        key_end,
        value: content[value_start..value_end].to_string(),
        value_start,
        value_end,
        table,
    })
}

fn frontmatter_completion_context(
    content: &str,
    position: Position,
) -> Option<FrontmatterCompletionContext> {
    let block = frontmatter_content_byte_range(content)?;
    let offset = position_to_byte_offset(content, position)?;
    if offset < block.start || offset > block.end {
        return None;
    }

    let (line_start, line_end) = line_bounds_at_offset(content, offset);
    let replace_start = scan_frontmatter_key_start(content, line_start, offset);
    let replace_end = scan_frontmatter_key_end(content, offset, line_end);
    let current_table = frontmatter_table_at_offset(content, offset);
    let mut present_fields = frontmatter_entries(content)
        .into_iter()
        .filter(|entry| entry.table.is_none())
        .map(|entry| entry.key)
        .collect::<HashSet<_>>();

    if frontmatter_has_extra_table(content) {
        present_fields.insert("extra".to_string());
    }

    Some(FrontmatterCompletionContext {
        replace_range: byte_range_to_lsp_range(content, replace_start, replace_end),
        present_fields,
        current_table,
    })
}

fn completion_items_for_frontmatter(
    source_file: &str,
    context: &FrontmatterCompletionContext,
) -> Vec<CompletionItem> {
    if context.current_table.is_some() {
        return Vec::new();
    }

    frontmatter_field_specs()
        .into_iter()
        .filter(|spec| !context.present_fields.contains(spec.name))
        .map(|spec| CompletionItem {
            label: frontmatter_completion_label(spec).to_string(),
            kind: Some(match spec.kind {
                FrontmatterFieldKind::Table => CompletionItemKind::STRUCT,
                _ => CompletionItemKind::FIELD,
            }),
            detail: Some(format!("Dodeca frontmatter {}", spec.kind.description())),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                context.replace_range,
                frontmatter_completion_text(source_file, spec),
            ))),
            ..CompletionItem::default()
        })
        .collect()
}

fn frontmatter_document_target_at_position(
    project: &AuthoringProject,
    content: &str,
    position: Position,
) -> Result<Option<FrontmatterDocumentTarget>> {
    Ok(frontmatter_document_targets(project, content)?
        .into_iter()
        .find(|target| range_contains_position(&target.source_range, position)))
}

fn frontmatter_document_targets(
    project: &AuthoringProject,
    content: &str,
) -> Result<Vec<FrontmatterDocumentTarget>> {
    let targets = frontmatter_entries(content)
        .into_iter()
        .filter_map(|entry| {
            let kind = frontmatter_document_kind_for_entry(&entry)?;
            let (path, source_range) = frontmatter_string_value(content, &entry)?;
            let target_path = match kind {
                FrontmatterDocumentKind::Template => project.template_paths.get(&path)?,
                FrontmatterDocumentKind::StaticAsset => {
                    frontmatter_static_target_path(project, &path)?
                }
                FrontmatterDocumentKind::DataFile => frontmatter_data_target_path(project, &path)?,
            };
            Some(FrontmatterDocumentTarget {
                kind,
                path,
                target_path: target_path.clone(),
                source_range,
            })
        })
        .collect();

    Ok(targets)
}

fn frontmatter_document_kind_for_entry(
    entry: &FrontmatterEntry,
) -> Option<FrontmatterDocumentKind> {
    if entry.table.is_some() {
        return None;
    }

    match entry.key.as_str() {
        "template" => Some(FrontmatterDocumentKind::Template),
        "asset" => Some(FrontmatterDocumentKind::StaticAsset),
        "data" => Some(FrontmatterDocumentKind::DataFile),
        _ => None,
    }
}

fn frontmatter_static_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .static_paths
        .get(trimmed)
        .or_else(|| project.static_paths.get(path))
}

fn frontmatter_data_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .data_paths
        .get(trimmed)
        .or_else(|| project.data_paths.get(path))
}

fn template_document_references(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    target_path: &Utf8Path,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();

    for (template_file, content) in &project.template_contents {
        for target in template_document_targets(project, template_file, content)? {
            if target.target_path == target_path {
                let Some(path) = project.template_paths.get(template_file) else {
                    continue;
                };
                locations.push(Location {
                    uri: Url::from_file_path(path.as_std_path())
                        .map_err(|_| eyre!("could not convert template path to URI: {path}"))?,
                    range: target.source_range,
                });
            }
        }
    }

    for (source_file, content) in &project.source_contents {
        for target in frontmatter_document_targets(project, content)? {
            if target.kind == FrontmatterDocumentKind::Template && target.target_path == target_path
            {
                let path = content_dir.join(source_file);
                locations.push(Location {
                    uri: Url::from_file_path(path.as_std_path())
                        .map_err(|_| eyre!("could not convert source path to URI: {path}"))?,
                    range: target.source_range,
                });
            }
        }
    }

    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    locations.dedup_by(|left, right| left.uri == right.uri && left.range == right.range);
    Ok(locations)
}

fn frontmatter_string_value(content: &str, entry: &FrontmatterEntry) -> Option<(String, Range)> {
    let value = entry.value.trim();
    let leading = entry.value.find(value)?;
    let start = entry.value_start + leading;
    let quote = value.as_bytes().first().copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    if value.as_bytes().last().copied()? != quote || value.len() < 2 {
        return None;
    }

    let inner_start = start + 1;
    let inner_end = start + value.len() - 1;
    Some((
        content[inner_start..inner_end].to_string(),
        byte_range_to_lsp_range(content, inner_start, inner_end),
    ))
}

fn template_document_target_at_position(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Result<Option<TemplateDocumentTarget>> {
    Ok(template_document_targets(project, template_file, content)?
        .into_iter()
        .find(|target| range_contains_position(&target.source_range, position)))
}

fn template_document_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Result<Vec<TemplateDocumentTarget>> {
    if !project.template_contents.contains_key(template_file) {
        return Ok(Vec::new());
    }
    let Ok(template) = TemplateParser::new(template_file, content).parse() else {
        return Ok(Vec::new());
    };

    let mut targets = Vec::new();
    collect_template_document_targets(project, content, &template.body, &mut targets);
    Ok(targets)
}

fn collect_template_document_targets(
    project: &AuthoringProject,
    content: &str,
    nodes: &[Node],
    targets: &mut Vec<TemplateDocumentTarget>,
) {
    for node in nodes {
        match node {
            Node::Extends(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Extends,
                &node.path,
                targets,
            ),
            Node::Include(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Include,
                &node.path,
                targets,
            ),
            Node::Import(node) => push_template_document_target(
                project,
                content,
                TemplateDocumentKind::Import,
                &node.path,
                targets,
            ),
            Node::If(node) => {
                collect_template_document_targets(project, content, &node.then_body, targets);
                for branch in &node.elif_branches {
                    collect_template_document_targets(project, content, &branch.body, targets);
                }
                if let Some(body) = &node.else_body {
                    collect_template_document_targets(project, content, body, targets);
                }
            }
            Node::For(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
                if let Some(body) = &node.else_body {
                    collect_template_document_targets(project, content, body, targets);
                }
            }
            Node::Block(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
            }
            Node::Macro(node) => {
                collect_template_document_targets(project, content, &node.body, targets);
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

fn push_template_document_target(
    project: &AuthoringProject,
    content: &str,
    kind: TemplateDocumentKind,
    path: &StringLit,
    targets: &mut Vec<TemplateDocumentTarget>,
) {
    let Some(target_path) = project.template_paths.get(&path.value) else {
        return;
    };
    targets.push(TemplateDocumentTarget {
        kind,
        path: path.value.clone(),
        target_path: target_path.clone(),
        source_range: template_string_range(content, path),
    });
}

fn template_string_range(content: &str, string: &StringLit) -> Range {
    byte_range_to_lsp_range(
        content,
        string.span.offset(),
        string.span.offset() + string.span.len(),
    )
}

fn template_ident_range(content: &str, ident: &Ident) -> Range {
    byte_range_to_lsp_range(
        content,
        ident.span.offset(),
        ident.span.offset() + ident.span.len(),
    )
}

#[allow(deprecated)]
fn template_document_symbols(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Result<Vec<DocumentSymbol>> {
    if !project.template_contents.contains_key(template_file) {
        return Ok(Vec::new());
    }
    let Ok(template) = TemplateParser::new(template_file, content).parse() else {
        return Ok(Vec::new());
    };

    let mut symbols = Vec::new();
    collect_template_document_symbols(content, &template.body, &mut symbols);
    Ok(symbols)
}

#[allow(deprecated)]
fn collect_template_document_symbols(
    content: &str,
    nodes: &[Node],
    symbols: &mut Vec<DocumentSymbol>,
) {
    for node in nodes {
        match node {
            Node::Block(node) => {
                let mut children = Vec::new();
                collect_template_document_symbols(content, &node.body, &mut children);
                symbols.push(DocumentSymbol {
                    name: node.name.name.clone(),
                    detail: Some("Dodeca template block".to_string()),
                    kind: SymbolKind::MODULE,
                    tags: None,
                    deprecated: None,
                    range: byte_range_to_lsp_range(
                        content,
                        node.span.offset(),
                        node.span.offset() + node.span.len(),
                    ),
                    selection_range: template_ident_range(content, &node.name),
                    children: (!children.is_empty()).then_some(children),
                });
            }
            Node::Macro(node) => {
                let mut children = Vec::new();
                collect_template_document_symbols(content, &node.body, &mut children);
                symbols.push(DocumentSymbol {
                    name: node.name.name.clone(),
                    detail: Some("Dodeca template macro".to_string()),
                    kind: SymbolKind::FUNCTION,
                    tags: None,
                    deprecated: None,
                    range: byte_range_to_lsp_range(
                        content,
                        node.span.offset(),
                        node.span.offset() + node.span.len(),
                    ),
                    selection_range: template_ident_range(content, &node.name),
                    children: (!children.is_empty()).then_some(children),
                });
            }
            Node::If(node) => {
                collect_template_document_symbols(content, &node.then_body, symbols);
                for branch in &node.elif_branches {
                    collect_template_document_symbols(content, &branch.body, symbols);
                }
                if let Some(body) = &node.else_body {
                    collect_template_document_symbols(content, body, symbols);
                }
            }
            Node::For(node) => {
                collect_template_document_symbols(content, &node.body, symbols);
                if let Some(body) = &node.else_body {
                    collect_template_document_symbols(content, body, symbols);
                }
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

fn template_block_occurrences(content: &str, nodes: &[Node]) -> Vec<TemplateBlockOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_block_occurrences(content, nodes, &mut occurrences);
    occurrences
}

fn collect_template_block_occurrences(
    content: &str,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateBlockOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Block(node) => {
                occurrences.push(TemplateBlockOccurrence {
                    name: node.name.name.clone(),
                    source_range: template_ident_range(content, &node.name),
                });
                collect_template_block_occurrences(content, &node.body, occurrences);
            }
            Node::If(node) => {
                collect_template_block_occurrences(content, &node.then_body, occurrences);
                for branch in &node.elif_branches {
                    collect_template_block_occurrences(content, &branch.body, occurrences);
                }
                if let Some(body) = &node.else_body {
                    collect_template_block_occurrences(content, body, occurrences);
                }
            }
            Node::For(node) => {
                collect_template_block_occurrences(content, &node.body, occurrences);
                if let Some(body) = &node.else_body {
                    collect_template_block_occurrences(content, body, occurrences);
                }
            }
            Node::Macro(node) => {
                collect_template_block_occurrences(content, &node.body, occurrences)
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

impl TemplateAuthoringIndex {
    fn new(project: &AuthoringProject) -> Self {
        let mut templates = HashMap::new();

        for (template_file, template_path) in &project.template_paths {
            let Some(content) = project.template_contents.get(template_file) else {
                continue;
            };
            let Ok(template) = TemplateParser::new(template_file, content).parse() else {
                continue;
            };
            let imports = template_import_aliases(project, &template.body);
            templates.insert(
                template_file.clone(),
                IndexedTemplate {
                    path: template_path.clone(),
                    extends: template_extends_path_from_nodes(&template.body),
                    blocks: template_block_occurrences(content, &template.body),
                    macros: template_macro_occurrences(content, &template.body),
                    macro_calls: template_macro_call_occurrences(
                        template_file,
                        content,
                        &imports,
                        &template.body,
                    ),
                },
            );
        }

        let mut children_by_parent = HashMap::<String, Vec<String>>::new();
        for (template_file, template) in &templates {
            let Some(parent_file) = &template.extends else {
                continue;
            };
            if templates.contains_key(parent_file) {
                children_by_parent
                    .entry(parent_file.clone())
                    .or_default()
                    .push(template_file.clone());
            }
        }
        for children in children_by_parent.values_mut() {
            children.sort();
        }

        Self {
            templates,
            children_by_parent,
        }
    }

    fn block_occurrence_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateBlockOccurrence> {
        self.templates
            .get(template_file)?
            .blocks
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
            .cloned()
    }

    fn block_definition_target(
        &self,
        template_file: &str,
        occurrence: &TemplateBlockOccurrence,
    ) -> Option<TemplateDefinitionTarget> {
        let mut seen = HashSet::new();
        let mut cursor = template_file.to_string();
        while seen.insert(cursor.clone()) {
            let parent_file = self.templates.get(&cursor)?.extends.clone()?;
            let parent = self.templates.get(&parent_file)?;
            if let Some(target) = parent
                .blocks
                .iter()
                .find(|block| block.name == occurrence.name)
            {
                return Some(TemplateDefinitionTarget {
                    kind: TemplateDefinitionKind::Block,
                    name: occurrence.name.clone(),
                    source_range: occurrence.source_range,
                    target_path: parent.path.clone(),
                    target_range: target.source_range,
                });
            }
            cursor = parent_file;
        }
        None
    }

    fn block_reference_targets(
        &self,
        template_file: &str,
        block_name: &str,
    ) -> Vec<TemplateBlockReferenceTarget> {
        let Some(owner) = self.block_reference_owner(template_file, block_name) else {
            return Vec::new();
        };

        let mut targets = Vec::new();
        self.collect_block_reference_targets(&owner, block_name, &mut targets);
        targets.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| position_cmp(a.range.start, b.range.start))
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        targets.dedup();
        targets
    }

    fn block_reference_owner(&self, template_file: &str, block_name: &str) -> Option<String> {
        let mut owner = self
            .template_declares_block(template_file, block_name)
            .then(|| template_file.to_string())?;
        let mut seen = HashSet::new();
        let mut cursor = template_file.to_string();

        while seen.insert(cursor.clone()) {
            let Some(parent_file) = self.templates.get(&cursor).and_then(|template| {
                template
                    .extends
                    .as_ref()
                    .filter(|parent| self.templates.contains_key(*parent))
                    .cloned()
            }) else {
                break;
            };
            if self.template_declares_block(&parent_file, block_name) {
                owner = parent_file.clone();
            }
            cursor = parent_file;
        }

        Some(owner)
    }

    fn collect_block_reference_targets(
        &self,
        template_file: &str,
        block_name: &str,
        targets: &mut Vec<TemplateBlockReferenceTarget>,
    ) {
        let Some(template) = self.templates.get(template_file) else {
            return;
        };
        targets.extend(
            template
                .blocks
                .iter()
                .filter(|block| block.name == block_name)
                .map(|block| TemplateBlockReferenceTarget {
                    path: template.path.clone(),
                    range: block.source_range,
                }),
        );
        if let Some(children) = self.children_by_parent.get(template_file) {
            for child in children {
                self.collect_block_reference_targets(child, block_name, targets);
            }
        }
    }

    fn template_declares_block(&self, template_file: &str, block_name: &str) -> bool {
        self.templates
            .get(template_file)
            .is_some_and(|template| template.blocks.iter().any(|block| block.name == block_name))
    }

    fn macro_reference_query(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateMacroReferenceQuery> {
        let template = self.templates.get(template_file)?;
        if let Some(occurrence) = template
            .macros
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
        {
            return Some(TemplateMacroReferenceQuery {
                target_template_file: template_file.to_string(),
                macro_name: occurrence.name.clone(),
            });
        }
        template
            .macro_calls
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
            .map(|occurrence| TemplateMacroReferenceQuery {
                target_template_file: occurrence.target_template_file.clone(),
                macro_name: occurrence.macro_name.clone(),
            })
    }

    fn macro_reference_targets(
        &self,
        target_template_file: &str,
        macro_name: &str,
    ) -> Vec<TemplateMacroReferenceTarget> {
        let mut targets = Vec::new();
        if let Some(template) = self.templates.get(target_template_file) {
            targets.extend(
                template
                    .macros
                    .iter()
                    .filter(|occurrence| occurrence.name == macro_name)
                    .map(|occurrence| TemplateMacroReferenceTarget {
                        path: template.path.clone(),
                        range: occurrence.source_range,
                    }),
            );
        }

        for template in self.templates.values() {
            targets.extend(
                template
                    .macro_calls
                    .iter()
                    .filter(|occurrence| {
                        occurrence.target_template_file == target_template_file
                            && occurrence.macro_name == macro_name
                    })
                    .map(|occurrence| TemplateMacroReferenceTarget {
                        path: template.path.clone(),
                        range: occurrence.source_range,
                    }),
            );
        }

        targets.sort_by(|a, b| {
            a.path
                .cmp(&b.path)
                .then_with(|| position_cmp(a.range.start, b.range.start))
                .then_with(|| position_cmp(a.range.end, b.range.end))
        });
        targets.dedup();
        targets
    }
}

fn template_block_hover_markdown(
    index: &TemplateAuthoringIndex,
    template_file: &str,
    occurrence: &TemplateBlockOccurrence,
) -> String {
    let mut sections = vec![format!("**Dodeca template block** `{}`", occurrence.name)];
    if let Some(target) = index.block_definition_target(template_file, occurrence) {
        sections.push(format!(
            "Overrides `{}` in `{}`.",
            occurrence.name, target.target_path
        ));
    } else {
        sections.push("Defines a block that child templates may override.".to_string());
    }
    let reference_count = index
        .block_reference_targets(template_file, &occurrence.name)
        .len();
    sections.push(format!("{reference_count} matching block declaration(s)."));
    sections.join("\n\n")
}

fn template_block_references(
    index: &TemplateAuthoringIndex,
    template_file: &str,
    block_name: &str,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();
    for target in index.block_reference_targets(template_file, block_name) {
        locations.push(Location {
            uri: Url::from_file_path(target.path.as_std_path()).map_err(|_| {
                eyre!(
                    "could not convert template block reference path to URI: {}",
                    target.path
                )
            })?,
            range: target.range,
        });
    }
    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    locations.dedup_by(|left, right| left.uri == right.uri && left.range == right.range);
    Ok(locations)
}

fn template_macro_references(
    index: &TemplateAuthoringIndex,
    target_template_file: &str,
    macro_name: &str,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();
    for target in index.macro_reference_targets(target_template_file, macro_name) {
        locations.push(Location {
            uri: Url::from_file_path(target.path.as_std_path()).map_err(|_| {
                eyre!(
                    "could not convert template macro reference path to URI: {}",
                    target.path
                )
            })?,
            range: target.range,
        });
    }
    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    locations.dedup_by(|left, right| left.uri == right.uri && left.range == right.range);
    Ok(locations)
}

fn template_macro_occurrences(content: &str, nodes: &[Node]) -> Vec<TemplateMacroOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_macro_occurrences(content, nodes, &mut occurrences);
    occurrences
}

fn collect_template_macro_occurrences(
    content: &str,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateMacroOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Macro(node) => {
                occurrences.push(TemplateMacroOccurrence {
                    name: node.name.name.clone(),
                    source_range: template_ident_range(content, &node.name),
                });
                collect_template_macro_occurrences(content, &node.body, occurrences);
            }
            Node::If(node) => {
                collect_template_macro_occurrences(content, &node.then_body, occurrences);
                for branch in &node.elif_branches {
                    collect_template_macro_occurrences(content, &branch.body, occurrences);
                }
                if let Some(body) = &node.else_body {
                    collect_template_macro_occurrences(content, body, occurrences);
                }
            }
            Node::For(node) => {
                collect_template_macro_occurrences(content, &node.body, occurrences);
                if let Some(body) = &node.else_body {
                    collect_template_macro_occurrences(content, body, occurrences);
                }
            }
            Node::Block(node) => {
                collect_template_macro_occurrences(content, &node.body, occurrences);
            }
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

fn template_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    nodes: &[Node],
) -> Vec<TemplateMacroCallOccurrence> {
    let mut occurrences = Vec::new();
    collect_template_macro_call_occurrences(
        template_file,
        content,
        imports,
        nodes,
        &mut occurrences,
    );
    occurrences
}

fn collect_template_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    nodes: &[Node],
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    for node in nodes {
        match node {
            Node::Print(node) => collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.expr,
                occurrences,
            ),
            Node::If(node) => {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.condition,
                    occurrences,
                );
                collect_template_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.then_body,
                    occurrences,
                );
                for branch in &node.elif_branches {
                    collect_expr_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        &branch.condition,
                        occurrences,
                    );
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        &branch.body,
                        occurrences,
                    );
                }
                if let Some(body) = &node.else_body {
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        body,
                        occurrences,
                    );
                }
            }
            Node::For(node) => {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.iter,
                    occurrences,
                );
                collect_template_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    &node.body,
                    occurrences,
                );
                if let Some(body) = &node.else_body {
                    collect_template_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        body,
                        occurrences,
                    );
                }
            }
            Node::Block(node) => collect_template_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.body,
                occurrences,
            ),
            Node::Set(node) => collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.value,
                occurrences,
            ),
            Node::Macro(node) => collect_template_macro_call_occurrences(
                template_file,
                content,
                imports,
                &node.body,
                occurrences,
            ),
            Node::CallBlock(node) => {
                for (_, expr) in &node.kwargs {
                    collect_expr_macro_call_occurrences(
                        template_file,
                        content,
                        imports,
                        expr,
                        occurrences,
                    );
                }
            }
            Node::Text(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_) => {}
        }
    }
}

fn collect_expr_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    expr: &Expr,
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    match expr {
        Expr::MacroCall(expr) => {
            let target_file = if expr.namespace.name == "self" {
                Some(template_file)
            } else {
                imports.get(&expr.namespace.name).map(|path| path.as_str())
            };
            if let Some(target_file) = target_file {
                occurrences.push(TemplateMacroCallOccurrence {
                    target_template_file: target_file.to_string(),
                    macro_name: expr.macro_name.name.clone(),
                    source_range: template_ident_range(content, &expr.macro_name),
                });
            }
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Field(expr) => collect_expr_macro_call_occurrences(
            template_file,
            content,
            imports,
            &expr.base,
            occurrences,
        ),
        Expr::Index(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.base,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.index,
                occurrences,
            );
        }
        Expr::Filter(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.expr,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Binary(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.left,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.right,
                occurrences,
            );
        }
        Expr::Unary(expr) => collect_expr_macro_call_occurrences(
            template_file,
            content,
            imports,
            &expr.expr,
            occurrences,
        ),
        Expr::Call(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.func,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        Expr::Ternary(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.value,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.condition,
                occurrences,
            );
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.otherwise,
                occurrences,
            );
        }
        Expr::Test(expr) => {
            collect_expr_macro_call_occurrences(
                template_file,
                content,
                imports,
                &expr.expr,
                occurrences,
            );
            for arg in &expr.args {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    arg,
                    occurrences,
                );
            }
        }
        Expr::Literal(literal) => {
            collect_literal_macro_call_occurrences(
                template_file,
                content,
                imports,
                literal,
                occurrences,
            );
        }
        Expr::Var(_) => {}
    }
}

fn collect_literal_macro_call_occurrences(
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    literal: &gingembre::ast::Literal,
    occurrences: &mut Vec<TemplateMacroCallOccurrence>,
) {
    match literal {
        gingembre::ast::Literal::List(list) => {
            for expr in &list.elements {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    expr,
                    occurrences,
                );
            }
        }
        gingembre::ast::Literal::Dict(dict) => {
            for (key, value) in &dict.entries {
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    key,
                    occurrences,
                );
                collect_expr_macro_call_occurrences(
                    template_file,
                    content,
                    imports,
                    value,
                    occurrences,
                );
            }
        }
        gingembre::ast::Literal::String(_)
        | gingembre::ast::Literal::Int(_)
        | gingembre::ast::Literal::Float(_)
        | gingembre::ast::Literal::Bool(_)
        | gingembre::ast::Literal::None(_) => {}
    }
}

fn template_definition_target_at_position(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    position: Position,
) -> Result<Option<TemplateDefinitionTarget>> {
    Ok(
        template_definition_targets(project, template_file, content)?
            .into_iter()
            .find(|target| range_contains_position(&target.source_range, position)),
    )
}

fn template_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Result<Vec<TemplateDefinitionTarget>> {
    if !project.template_contents.contains_key(template_file) {
        return Ok(Vec::new());
    }
    let Ok(template) = TemplateParser::new(template_file, content).parse() else {
        return Ok(Vec::new());
    };

    let imports = template_import_aliases(project, &template.body);
    let mut targets = Vec::new();
    collect_template_definition_targets(
        project,
        template_file,
        content,
        &template.body,
        &imports,
        &mut targets,
    );
    Ok(targets)
}

fn collect_template_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    nodes: &[Node],
    imports: &HashMap<String, String>,
    targets: &mut Vec<TemplateDefinitionTarget>,
) {
    for node in nodes {
        match node {
            Node::Block(node) => {
                if let Some(target) =
                    template_block_definition_target(project, template_file, content, &node.name)
                {
                    targets.push(target);
                }
                collect_template_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.body,
                    imports,
                    targets,
                );
            }
            Node::Macro(node) => {
                collect_template_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.body,
                    imports,
                    targets,
                );
            }
            Node::Print(node) => collect_expr_definition_targets(
                project,
                template_file,
                content,
                &node.expr,
                imports,
                targets,
            ),
            Node::If(node) => {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.condition,
                    imports,
                    targets,
                );
                collect_template_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.then_body,
                    imports,
                    targets,
                );
                for branch in &node.elif_branches {
                    collect_expr_definition_targets(
                        project,
                        template_file,
                        content,
                        &branch.condition,
                        imports,
                        targets,
                    );
                    collect_template_definition_targets(
                        project,
                        template_file,
                        content,
                        &branch.body,
                        imports,
                        targets,
                    );
                }
                if let Some(body) = &node.else_body {
                    collect_template_definition_targets(
                        project,
                        template_file,
                        content,
                        body,
                        imports,
                        targets,
                    );
                }
            }
            Node::For(node) => {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.iter,
                    imports,
                    targets,
                );
                collect_template_definition_targets(
                    project,
                    template_file,
                    content,
                    &node.body,
                    imports,
                    targets,
                );
                if let Some(body) = &node.else_body {
                    collect_template_definition_targets(
                        project,
                        template_file,
                        content,
                        body,
                        imports,
                        targets,
                    );
                }
            }
            Node::Set(node) => collect_expr_definition_targets(
                project,
                template_file,
                content,
                &node.value,
                imports,
                targets,
            ),
            Node::CallBlock(node) => {
                for (_, expr) in &node.kwargs {
                    collect_expr_definition_targets(
                        project,
                        template_file,
                        content,
                        expr,
                        imports,
                        targets,
                    );
                }
            }
            Node::Text(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Import(_)
            | Node::Continue(_)
            | Node::Break(_) => {}
        }
    }
}

fn collect_expr_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    expr: &Expr,
    imports: &HashMap<String, String>,
    targets: &mut Vec<TemplateDefinitionTarget>,
) {
    match expr {
        Expr::Literal(literal) => collect_literal_definition_targets(
            project,
            template_file,
            content,
            literal,
            imports,
            targets,
        ),
        Expr::Var(_) => {}
        Expr::Field(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                targets,
            );
        }
        Expr::Index(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.base,
                imports,
                targets,
            );
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.index,
                imports,
                targets,
            );
        }
        Expr::Filter(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                targets,
            );
            for arg in &expr.args {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    targets,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    targets,
                );
            }
            if builtin_filter(&expr.filter.name).is_some()
                && let Some(target) = builtin_template_definition_target(
                    TemplateDefinitionKind::Filter,
                    &expr.filter,
                    content,
                )
            {
                targets.push(target);
            }
        }
        Expr::Binary(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.left,
                imports,
                targets,
            );
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.right,
                imports,
                targets,
            );
        }
        Expr::Unary(expr) => collect_expr_definition_targets(
            project,
            template_file,
            content,
            &expr.expr,
            imports,
            targets,
        ),
        Expr::Call(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.func,
                imports,
                targets,
            );
            for arg in &expr.args {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    targets,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    targets,
                );
            }
        }
        Expr::Ternary(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.value,
                imports,
                targets,
            );
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.condition,
                imports,
                targets,
            );
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.otherwise,
                imports,
                targets,
            );
        }
        Expr::Test(expr) => {
            collect_expr_definition_targets(
                project,
                template_file,
                content,
                &expr.expr,
                imports,
                targets,
            );
            for arg in &expr.args {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    targets,
                );
            }
            if builtin_test(&expr.test_name.name).is_some()
                && let Some(target) = builtin_template_definition_target(
                    TemplateDefinitionKind::Test,
                    &expr.test_name,
                    content,
                )
            {
                targets.push(target);
            }
        }
        Expr::MacroCall(expr) => {
            for arg in &expr.args {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    arg,
                    imports,
                    targets,
                );
            }
            for (_, expr) in &expr.kwargs {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    targets,
                );
            }
            if let Some(target) = template_macro_definition_target(
                project,
                template_file,
                content,
                imports,
                &expr.namespace,
                &expr.macro_name,
            ) {
                targets.push(target);
            }
        }
    }
}

fn collect_literal_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    literal: &gingembre::ast::Literal,
    imports: &HashMap<String, String>,
    targets: &mut Vec<TemplateDefinitionTarget>,
) {
    match literal {
        gingembre::ast::Literal::List(list) => {
            for expr in &list.elements {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    expr,
                    imports,
                    targets,
                );
            }
        }
        gingembre::ast::Literal::Dict(dict) => {
            for (key, value) in &dict.entries {
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    key,
                    imports,
                    targets,
                );
                collect_expr_definition_targets(
                    project,
                    template_file,
                    content,
                    value,
                    imports,
                    targets,
                );
            }
        }
        gingembre::ast::Literal::String(_)
        | gingembre::ast::Literal::Int(_)
        | gingembre::ast::Literal::Float(_)
        | gingembre::ast::Literal::Bool(_)
        | gingembre::ast::Literal::None(_) => {}
    }
}

fn template_import_aliases(project: &AuthoringProject, nodes: &[Node]) -> HashMap<String, String> {
    let mut imports = HashMap::new();
    collect_template_import_aliases(project, nodes, &mut imports);
    imports
}

fn collect_template_import_aliases(
    project: &AuthoringProject,
    nodes: &[Node],
    imports: &mut HashMap<String, String>,
) {
    for node in nodes {
        match node {
            Node::Import(node) => {
                if project.template_paths.contains_key(&node.path.value) {
                    imports.insert(node.alias.name.clone(), node.path.value.clone());
                }
            }
            Node::If(node) => {
                collect_template_import_aliases(project, &node.then_body, imports);
                for branch in &node.elif_branches {
                    collect_template_import_aliases(project, &branch.body, imports);
                }
                if let Some(body) = &node.else_body {
                    collect_template_import_aliases(project, body, imports);
                }
            }
            Node::For(node) => {
                collect_template_import_aliases(project, &node.body, imports);
                if let Some(body) = &node.else_body {
                    collect_template_import_aliases(project, body, imports);
                }
            }
            Node::Block(node) => collect_template_import_aliases(project, &node.body, imports),
            Node::Macro(node) => collect_template_import_aliases(project, &node.body, imports),
            Node::Text(_)
            | Node::Print(_)
            | Node::Include(_)
            | Node::Extends(_)
            | Node::Comment(_)
            | Node::Set(_)
            | Node::Continue(_)
            | Node::Break(_)
            | Node::CallBlock(_) => {}
        }
    }
}

fn template_block_definition_target(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    source_ident: &Ident,
) -> Option<TemplateDefinitionTarget> {
    let mut seen = HashSet::new();
    let parent_file = template_extends_path(template_file, content, &mut seen)?;
    let (target_file, target_content, target_ident) = template_block_definition(
        project,
        &parent_file,
        template_file,
        content,
        &source_ident.name,
    )?;
    Some(TemplateDefinitionTarget {
        kind: TemplateDefinitionKind::Block,
        name: source_ident.name.clone(),
        source_range: template_ident_range(content, source_ident),
        target_path: project.template_paths.get(&target_file).cloned()?,
        target_range: template_ident_range(&target_content, &target_ident),
    })
}

fn template_block_definition(
    project: &AuthoringProject,
    template_file: &str,
    current_file: &str,
    current_content: &str,
    name: &str,
) -> Option<(String, String, Ident)> {
    let content = template_content(project, template_file, current_file, current_content)?;
    let template = TemplateParser::new(template_file, content.as_str())
        .parse()
        .ok()?;
    if let Some(ident) = top_level_block_ident(&template.body, name) {
        return Some((template_file.to_string(), content, ident));
    }

    let mut seen = HashSet::new();
    if let Some(parent_file) = template_extends_path(template_file, &content, &mut seen) {
        return template_block_definition(
            project,
            &parent_file,
            current_file,
            current_content,
            name,
        );
    }

    None
}

fn top_level_block_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Block(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

fn template_extends_path(
    template_file: &str,
    content: &str,
    seen: &mut HashSet<String>,
) -> Option<String> {
    if !seen.insert(template_file.to_string()) {
        return None;
    }
    let template = TemplateParser::new(template_file, content).parse().ok()?;
    template_extends_path_from_nodes(&template.body)
}

fn template_extends_path_from_nodes(nodes: &[Node]) -> Option<String> {
    for node in nodes {
        match node {
            Node::Extends(node) => return Some(node.path.value.clone()),
            Node::Text(node) if node.text.trim().is_empty() => {}
            _ => return None,
        }
    }
    None
}

fn template_macro_definition_target(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    imports: &HashMap<String, String>,
    namespace: &Ident,
    macro_name: &Ident,
) -> Option<TemplateDefinitionTarget> {
    let target_file = if namespace.name == "self" {
        template_file
    } else {
        imports.get(&namespace.name)?.as_str()
    };
    let target_content = template_content(project, target_file, template_file, content)?;
    let template = TemplateParser::new(target_file, target_content.as_str())
        .parse()
        .ok()?;
    let target_ident = top_level_macro_ident(&template.body, &macro_name.name)?;
    Some(TemplateDefinitionTarget {
        kind: TemplateDefinitionKind::Macro,
        name: format!("{}::{}", namespace.name, macro_name.name),
        source_range: template_ident_range(content, macro_name),
        target_path: project.template_paths.get(target_file).cloned()?,
        target_range: template_ident_range(&target_content, &target_ident),
    })
}

fn top_level_macro_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Macro(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

fn template_content(
    project: &AuthoringProject,
    template_file: &str,
    current_file: &str,
    current_content: &str,
) -> Option<String> {
    if template_file == current_file {
        Some(current_content.to_string())
    } else {
        project.template_contents.get(template_file).cloned()
    }
}

fn builtin_template_definition_target(
    kind: TemplateDefinitionKind,
    ident: &Ident,
    content: &str,
) -> Option<TemplateDefinitionTarget> {
    let target_path = gingembre_eval_source_path()?;
    let target_content = std::fs::read_to_string(target_path.as_std_path()).ok()?;
    let target_range = builtin_template_definition_range(&target_content, kind, &ident.name)?;
    Some(TemplateDefinitionTarget {
        kind,
        name: ident.name.clone(),
        source_range: template_ident_range(content, ident),
        target_path,
        target_range,
    })
}

fn gingembre_eval_source_path() -> Option<Utf8PathBuf> {
    let dodeca_manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Some(dodeca_manifest_dir.parent()?.join("gingembre/src/eval.rs"))
}

fn builtin_template_definition_range(
    source: &str,
    kind: TemplateDefinitionKind,
    name: &str,
) -> Option<Range> {
    let marker = match kind {
        TemplateDefinitionKind::Filter => "Ok(match name {",
        TemplateDefinitionKind::Test => "let result = match test.test_name.name.as_str() {",
        TemplateDefinitionKind::Block | TemplateDefinitionKind::Macro => return None,
    };
    let section_start = source.find(marker)?;
    let needle = format!("\"{name}\"");
    let name_start = section_start + source[section_start..].find(&needle)? + 1;
    Some(byte_range_to_lsp_range(
        source,
        name_start,
        name_start + name.len(),
    ))
}

fn frontmatter_completion_label(spec: FrontmatterFieldSpec) -> &'static str {
    match spec.kind {
        FrontmatterFieldKind::Table => "[extra]",
        _ => spec.name,
    }
}

fn frontmatter_completion_text(source_file: &str, spec: FrontmatterFieldSpec) -> String {
    match (spec.name, spec.kind) {
        ("title", FrontmatterFieldKind::String) => {
            format!(
                "title = \"{}\"",
                toml_basic_string_escape(&default_title_from_source_path(source_file))
            )
        }
        ("description", FrontmatterFieldKind::String) => "description = \"\"".to_string(),
        ("template", FrontmatterFieldKind::String) => {
            let default_template = if SourcePath::new(source_file.to_string()).is_section_index() {
                "section.html"
            } else {
                "page.html"
            };
            format!("template = \"{default_template}\"")
        }
        ("asset", FrontmatterFieldKind::String) => "asset = \"\"".to_string(),
        ("data", FrontmatterFieldKind::String) => "data = \"\"".to_string(),
        ("weight", FrontmatterFieldKind::Integer) => "weight = 0".to_string(),
        ("extra", FrontmatterFieldKind::Table) => "[extra]\n".to_string(),
        _ => spec.name.to_string(),
    }
}

fn frontmatter_value_matches_kind(value: &str, kind: FrontmatterFieldKind) -> bool {
    let value = value.trim();
    match kind {
        FrontmatterFieldKind::String => value.starts_with('"') || value.starts_with('\''),
        FrontmatterFieldKind::Integer => frontmatter_value_is_integer(value),
        FrontmatterFieldKind::Table => true,
    }
}

fn frontmatter_value_is_integer(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let mut previous_underscore = false;
    let mut saw_digit = false;

    for ch in value.chars() {
        match ch {
            '_' if saw_digit && !previous_underscore => previous_underscore = true,
            '0'..='9' => {
                saw_digit = true;
                previous_underscore = false;
            }
            _ => return false,
        }
    }

    saw_digit && !previous_underscore
}

#[derive(Debug, Clone, Copy)]
struct FrontmatterContentByteRange {
    start: usize,
    end: usize,
}

fn frontmatter_content_byte_range(content: &str) -> Option<FrontmatterContentByteRange> {
    content.strip_prefix("+++\n")?;
    let closing_start = content[4..].find("\n+++")? + 4;
    Some(FrontmatterContentByteRange {
        start: 4,
        end: closing_start,
    })
}

fn frontmatter_table_name(trimmed_line: &str) -> Option<String> {
    let inner = trimmed_line.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

fn frontmatter_table_at_offset(content: &str, offset: usize) -> Option<String> {
    let block = frontmatter_content_byte_range(content)?;
    let mut table = None;
    let mut line_start = block.start;
    let limit = offset.min(block.end);

    while line_start < limit {
        let line_end = content[line_start..limit]
            .find('\n')
            .map(|line_end| line_start + line_end)
            .unwrap_or(limit);
        if let Some(table_name) = frontmatter_table_name(content[line_start..line_end].trim()) {
            table = Some(table_name);
        }
        line_start = line_end.saturating_add(1);
    }

    table
}

fn frontmatter_has_extra_table(content: &str) -> bool {
    let Some(block) = frontmatter_content_byte_range(content) else {
        return false;
    };
    content[block.start..block.end]
        .lines()
        .filter_map(|line| frontmatter_table_name(line.trim()))
        .any(|table| table == "extra")
}

fn is_frontmatter_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn leading_whitespace_len(input: &str) -> usize {
    input
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(input.len())
}

fn trailing_whitespace_len(input: &str) -> usize {
    input.len() - input.trim_end_matches(char::is_whitespace).len()
}

fn line_comment_start(input: &str) -> Option<usize> {
    let mut in_string = false;
    let mut quote = '\0';
    let mut escaped = false;

    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' && quote == '"' {
                escaped = true;
            } else if ch == quote {
                in_string = false;
            }
        } else if ch == '"' || ch == '\'' {
            in_string = true;
            quote = ch;
        } else if ch == '#' {
            return Some(idx);
        }
    }

    None
}

fn line_bounds_at_offset(content: &str, offset: usize) -> (usize, usize) {
    let start = content[..offset]
        .rfind('\n')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let end = content[offset..]
        .find('\n')
        .map(|idx| offset + idx)
        .unwrap_or(content.len());
    (start, end)
}

fn scan_frontmatter_key_start(content: &str, line_start: usize, offset: usize) -> usize {
    let mut start = offset;
    while start > line_start {
        let previous = content.as_bytes()[start - 1] as char;
        if !is_frontmatter_key_char(previous) {
            break;
        }
        start -= 1;
    }
    start
}

fn scan_frontmatter_key_end(content: &str, offset: usize, line_end: usize) -> usize {
    let mut end = offset;
    while end < line_end {
        let next = content.as_bytes()[end] as char;
        if !is_frontmatter_key_char(next) {
            break;
        }
        end += 1;
    }
    end
}

fn source_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<String> {
    Ok(path
        .strip_prefix(content_dir)
        .map_err(|_| eyre!("content file is outside content root: {path}"))?
        .to_string())
}

fn template_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<Option<String>> {
    let project_dir = content_dir.parent().unwrap_or(content_dir);
    let templates_dir = project_dir.join("templates");
    match path.strip_prefix(&templates_dir) {
        Ok(relative) if path.extension() == Some("html") => Ok(Some(relative.to_string())),
        Ok(_) | Err(_) => Ok(None),
    }
}

fn is_content_markdown_document(content_dir: &Utf8Path, uri: &Url) -> bool {
    lsp_file_uri_to_utf8_path(uri)
        .ok()
        .filter(|path| path.extension() == Some("md"))
        .and_then(|path| source_file_for_path(content_dir, &path).ok())
        .is_some()
}

fn missing_anchor_message(
    project: &AuthoringProject,
    target_route: &str,
    fragment: Option<&str>,
) -> Option<String> {
    let fragment = fragment.filter(|fragment| !fragment.is_empty())?;
    if project.heading_exists(target_route, fragment)? {
        None
    } else {
        Some(format!("anchor '#{fragment}' not found on target page"))
    }
}

fn is_special_target(target: &str) -> bool {
    target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
        || target.starts_with("tel:")
        || target.starts_with("javascript:")
        || target.starts_with("data:")
        || target.starts_with("/__")
}

fn split_fragment(target: &str) -> (&str, Option<&str>) {
    let target = strip_query(target);
    match target.find('#') {
        Some(idx) => (&target[..idx], Some(&target[idx + 1..])),
        None => (target, None),
    }
}

fn strip_query(target: &str) -> &str {
    target.split('?').next().unwrap_or(target)
}

fn is_likely_static_file(path: &str) -> bool {
    let extensions = [
        ".css", ".js", ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".woff", ".woff2", ".ttf",
        ".eot", ".pdf", ".zip", ".tar", ".gz", ".webp", ".jxl", ".xml", ".txt", ".wasm",
    ];
    extensions.iter().any(|ext| path.ends_with(ext))
}

fn normalize_route(path: &str) -> String {
    let mut parts = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }

    if parts.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn byte_to_line_column(content: &str, byte_offset: usize) -> (u32, u32) {
    let mut line = 1;
    let mut column = 1;
    for (idx, ch) in content.char_indices() {
        if idx >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
    let target_line = position.line as usize;
    let target_character = position.character as usize;
    let line_start = content
        .split_inclusive('\n')
        .take(target_line)
        .map(str::len)
        .sum::<usize>();
    let line = content.split_inclusive('\n').nth(target_line)?;
    let line_without_newline = line.strip_suffix('\n').unwrap_or(line);

    if target_character == 0 {
        return Some(line_start);
    }

    let mut chars_seen = 0;
    for (offset, _) in line_without_newline.char_indices() {
        if chars_seen == target_character {
            return Some(line_start + offset);
        }
        chars_seen += 1;
    }

    (chars_seen == target_character).then_some(line_start + line_without_newline.len())
}

fn authoring_diagnostic_to_lsp(diagnostic: &AuthoringDiagnostic) -> Diagnostic {
    Diagnostic {
        range: diagnostic.range(),
        severity: Some(DiagnosticSeverity::WARNING),
        code: Some(NumberOrString::String(
            diagnostic_kind_name(diagnostic.kind).to_string(),
        )),
        source: Some("dodeca".to_string()),
        message: diagnostic.message.clone(),
        ..Default::default()
    }
}

fn ranges_overlap(left: &Range, right: &Range) -> bool {
    position_le(left.start, right.end) && position_le(right.start, left.end)
}

fn range_contains_position(range: &Range, position: Position) -> bool {
    position_le(range.start, position) && position_le(position, range.end)
}

fn position_le(left: Position, right: Position) -> bool {
    left.line < right.line || (left.line == right.line && left.character <= right.character)
}

fn position_cmp(left: Position, right: Position) -> std::cmp::Ordering {
    left.line
        .cmp(&right.line)
        .then_with(|| left.character.cmp(&right.character))
}

fn source_file_for_new_route(route: &str) -> Option<String> {
    let route = normalize_route(route);
    let relative = route.strip_prefix('/')?;
    if relative.is_empty() {
        return None;
    }

    let mut segments = Vec::new();
    for segment in relative.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            return None;
        }
        segments.push(segment);
    }

    Some(format!("{}.md", segments.join("/")))
}

fn page_frontmatter(title: &str) -> String {
    format!(
        "+++\ntitle = \"{}\"\n+++\n",
        toml_basic_string_escape(title)
    )
}

fn toml_basic_string_escape(input: &str) -> String {
    let mut escaped = String::new();
    for c in input.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => escaped.push(' '),
            c => escaped.push(c),
        }
    }
    escaped
}

// tower-lsp command arguments are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn create_page_command_arguments(source_uri: &Url, route: &str) -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "sourceUri": source_uri.as_str(),
        "route": route,
    })]
}

#[allow(clippy::disallowed_types)]
fn parse_create_page_command_arguments(arguments: &[serde_json::Value]) -> Result<(Url, String)> {
    let argument = arguments
        .first()
        .ok_or_else(|| eyre!("missing create page command arguments"))?;
    let source_uri = argument
        .get("sourceUri")
        .and_then(|value| value.as_str())
        .ok_or_else(|| eyre!("missing sourceUri create page argument"))?;
    let route = argument
        .get("route")
        .and_then(|value| value.as_str())
        .ok_or_else(|| eyre!("missing route create page argument"))?;
    Ok((Url::parse(source_uri)?, route.to_string()))
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn created_page_to_json(source_file: &str, route: &str, uri: &Url) -> serde_json::Value {
    serde_json::json!({
        "sourceFile": source_file,
        "route": route,
        "uri": uri.as_str(),
    })
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn pages_to_json(pages: &[AuthoringPage]) -> serde_json::Value {
    serde_json::Value::Array(
        pages
            .iter()
            .map(|page| {
                serde_json::json!({
                    "kind": match page.kind {
                        AuthoringPageKind::Page => "page",
                        AuthoringPageKind::Section => "section",
                    },
                    "route": page.route,
                    "sourceFile": page.source_file,
                    "title": page.title,
                    "headingIds": page.heading_ids,
                    "sourceSpan": {
                        "lineStart": 1,
                        "lineEnd": 1,
                    },
                })
            })
            .collect(),
    )
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn diagnostics_to_json(diagnostics: &[AuthoringDiagnostic]) -> serde_json::Value {
    serde_json::Value::Array(
        diagnostics
            .iter()
            .map(|diagnostic| {
                serde_json::json!({
                    "sourceFile": diagnostic.source_file,
                    "route": diagnostic.route,
                    "kind": diagnostic_kind_name(diagnostic.kind),
                    "target": diagnostic.target,
                    "resolvedRoute": diagnostic.resolved_route.as_deref(),
                    "message": diagnostic.message,
                    "span": {
                        "lineStart": diagnostic.line,
                        "lineEnd": diagnostic.line_end,
                        "columnStart": diagnostic.column,
                        "columnEnd": diagnostic.column_end,
                        "byteStart": diagnostic.byte_start,
                        "byteEnd": diagnostic.byte_end,
                    },
                })
            })
            .collect(),
    )
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn route_graph_to_json(graph: &[RouteGraphNode]) -> serde_json::Value {
    serde_json::Value::Array(
        graph
            .iter()
            .map(|node| {
                serde_json::json!({
                    "route": node.route,
                    "sourceFile": node.source_file,
                    "title": node.title,
                    "incoming": route_graph_edges_to_json(&node.incoming),
                    "outgoing": route_graph_edges_to_json(&node.outgoing),
                })
            })
            .collect(),
    )
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn route_graph_edges_to_json(edges: &[RouteGraphEdge]) -> serde_json::Value {
    serde_json::Value::Array(
        edges
            .iter()
            .map(|edge| {
                serde_json::json!({
                    "kind": edge.kind.label(),
                    "sourceRoute": edge.source_route,
                    "sourceFile": edge.source_file,
                    "targetRoute": edge.target_route,
                    "target": edge.target,
                    "span": route_graph_edge_span_to_json(edge),
                })
            })
            .collect(),
    )
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
fn route_graph_edge_span_to_json(edge: &RouteGraphEdge) -> serde_json::Value {
    match (edge.line, edge.line_end, edge.column, edge.column_end) {
        (Some(line), Some(line_end), Some(column), Some(column_end)) => {
            serde_json::json!({
                "lineStart": line,
                "lineEnd": line_end,
                "columnStart": column,
                "columnEnd": column_end,
            })
        }
        _ => serde_json::Value::Null,
    }
}

fn diagnostic_kind_name(kind: AuthoringDiagnosticKind) -> &'static str {
    match kind {
        AuthoringDiagnosticKind::Route => "missingRoute",
        AuthoringDiagnosticKind::Anchor => "missingAnchor",
        AuthoringDiagnosticKind::Source => "missingSource",
        AuthoringDiagnosticKind::StaticAsset => "missingStaticAsset",
        AuthoringDiagnosticKind::Frontmatter => "frontmatter",
        AuthoringDiagnosticKind::MissingTemplate => "missingTemplate",
        AuthoringDiagnosticKind::MissingBlock => "missingBlock",
        AuthoringDiagnosticKind::UnknownMacro => "unknownMacro",
        AuthoringDiagnosticKind::UnknownFilter => "unknownFilter",
        AuthoringDiagnosticKind::UnknownTest => "unknownTest",
        AuthoringDiagnosticKind::DuplicateTitle => "duplicateTitle",
        AuthoringDiagnosticKind::DuplicateRoute => "duplicateRoute",
        AuthoringDiagnosticKind::OrphanPage => "orphanPage",
        AuthoringDiagnosticKind::NoInboundLinks => "noInboundLinks",
    }
}

#[cfg(test)]
mod tests {
    use crate::authoring_model::AuthoringInputPath;

    use super::*;
    use crate::authoring_model::load_authoring_project;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower_lsp::lsp_types::{ClientCapabilities, WorkspaceFolder};

    fn temp_dir(name: &str) -> Utf8PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        Utf8PathBuf::from_path_buf(std::env::temp_dir().join(format!(
            "dodeca-authoring-lsp-{name}-{}-{nonce}",
            std::process::id()
        )))
        .expect("utf8 temp path")
    }

    fn position_for(content: &str, needle: &str) -> Position {
        let byte = content.find(needle).expect("needle in content");
        let (line, column) = byte_to_line_column(content, byte);
        Position {
            line: line - 1,
            character: column - 1,
        }
    }

    fn position_for_nth(content: &str, needle: &str, occurrence: usize) -> Position {
        let mut search_start = 0;
        for _ in 0..occurrence {
            let relative = content[search_start..]
                .find(needle)
                .expect("needle occurrence in content");
            search_start += relative + needle.len();
        }
        let relative = content[search_start..]
            .find(needle)
            .expect("needle occurrence in content");
        let byte = search_start + relative;
        let (line, column) = byte_to_line_column(content, byte);
        Position {
            line: line - 1,
            character: column - 1,
        }
    }

    fn range_for(content: &str, needle: &str) -> Range {
        let start_byte = content.find(needle).expect("needle in content");
        let end_byte = start_byte + needle.len();
        byte_range_to_lsp_range(content, start_byte, end_byte)
    }

    #[allow(deprecated)]
    fn initialize_params_for_workspace(project_dir: &Utf8Path) -> InitializeParams {
        let uri = Url::from_directory_path(project_dir.as_std_path()).expect("workspace uri");
        InitializeParams {
            process_id: None,
            root_path: None,
            root_uri: None,
            initialization_options: None,
            capabilities: ClientCapabilities::default(),
            trace: None,
            workspace_folders: Some(vec![WorkspaceFolder {
                uri,
                name: "dodeca-site".to_string(),
            }]),
            client_info: None,
            locale: None,
        }
    }

    #[allow(deprecated)]
    fn empty_initialize_params() -> InitializeParams {
        InitializeParams {
            process_id: None,
            root_path: None,
            root_uri: None,
            initialization_options: None,
            capabilities: ClientCapabilities::default(),
            trace: None,
            workspace_folders: None,
            client_info: None,
            locale: None,
        }
    }

    fn default_startup_args() -> LspStartupArgs {
        LspStartupArgs {
            content: None,
            output: None,
        }
    }

    #[test]
    fn maps_missing_route_to_new_page_source_file() {
        assert_eq!(source_file_for_new_route("/ij"), Some("ij.md".to_string()));
        assert_eq!(
            source_file_for_new_route("/ops/deploy"),
            Some("ops/deploy.md".to_string())
        );
        assert_eq!(source_file_for_new_route("/"), None);
        assert_eq!(source_file_for_new_route("/bad:route"), None);
    }

    #[test]
    fn creates_pages_with_frontmatter_only() {
        assert_eq!(
            page_frontmatter("New \"Page\""),
            "+++\ntitle = \"New \\\"Page\\\"\"\n+++\n"
        );
    }

    #[test]
    fn recognizes_frontmatter_as_page_identity_range() {
        let content = "+++\ntitle = \"Guide\"\n+++\n\nBody\n";
        let range = frontmatter_lsp_range(content).expect("frontmatter range");

        assert!(range_contains_position(
            &range,
            position_for(content, "title")
        ));
        assert!(!range_contains_position(
            &range,
            position_for(content, "Body")
        ));
    }

    #[test]
    fn validates_frontmatter_against_typed_fields() {
        let content = "+++\ntitl = \"Typo\"\ntitle = \"Guide\"\ntitle = \"Duplicate\"\nweight = \"heavy\"\ntemplate = 42\n[extra]\ncustom = true\n+++\n";
        let diagnostics = frontmatter_diagnostics_for_source("guide.md", "/guide", content);
        let messages = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>();

        assert!(messages.contains(&"unknown Dodeca frontmatter field 'titl'"));
        assert!(messages.contains(&"duplicate Dodeca frontmatter field 'title'"));
        assert!(messages.contains(&"frontmatter field 'weight' expects an integer"));
        assert!(messages.contains(&"frontmatter field 'template' expects a string"));
        assert!(!messages.iter().any(|message| message.contains("custom")));
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.kind == AuthoringDiagnosticKind::Frontmatter)
        );
    }

    #[test]
    fn completes_missing_frontmatter_fields_from_schema() {
        let content = "+++\ntitle = \"Guide\"\n\n+++\n";
        let context =
            frontmatter_completion_context(content, Position::new(2, 0)).expect("context");
        let items = completion_items_for_frontmatter("guide.md", &context);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();

        assert!(!labels.contains(&"title"));
        assert!(labels.contains(&"description"));
        assert!(labels.contains(&"weight"));
        assert!(labels.contains(&"template"));
        assert!(labels.contains(&"[extra]"));
        assert!(items.iter().any(|item| {
            item.text_edit.as_ref().is_some_and(|edit| match edit {
                CompletionTextEdit::Edit(edit) => edit.new_text == "template = \"page.html\"",
                CompletionTextEdit::InsertAndReplace(_) => false,
            })
        }));
    }

    #[tokio::test]
    async fn resolves_frontmatter_template_document_targets_from_authoring_model() {
        let dir = temp_dir("frontmatter-template-link");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        let static_dir = dir.join("static");
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::create_dir_all(&static_dir).expect("create static dir");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::write(templates_dir.join("custom.html"), "{{ page.content }}")
            .expect("write template");
        std::fs::write(static_dir.join("logo.png"), "png").expect("write static asset");
        std::fs::write(data_dir.join("versions.toml"), "stable = \"1.0\"").expect("write data");

        let content = "+++\ntitle = \"Guide\"\ntemplate = \"custom.html\"\nasset = \"/logo.png\"\ndata = \"versions.toml\"\n+++\n";
        std::fs::write(content_dir.join("guide.md"), content).expect("write page");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let targets = frontmatter_document_targets(&project, content).expect("targets");
        assert_eq!(targets.len(), 3);

        let template_target = targets
            .iter()
            .find(|target| target.kind == FrontmatterDocumentKind::Template)
            .expect("template target");
        assert_eq!(template_target.path, "custom.html");
        assert_eq!(
            template_target.target_path,
            templates_dir.join("custom.html")
        );
        assert!(range_contains_position(
            &template_target.source_range,
            position_for(content, "custom.html")
        ));

        let asset_target = targets
            .iter()
            .find(|target| target.kind == FrontmatterDocumentKind::StaticAsset)
            .expect("asset target");
        assert_eq!(asset_target.path, "/logo.png");
        assert_eq!(asset_target.target_path, static_dir.join("logo.png"));
        assert!(range_contains_position(
            &asset_target.source_range,
            position_for(content, "logo.png")
        ));

        let data_target = targets
            .iter()
            .find(|target| target.kind == FrontmatterDocumentKind::DataFile)
            .expect("data target");
        assert_eq!(data_target.path, "versions.toml");
        assert_eq!(data_target.target_path, data_dir.join("versions.toml"));
        assert!(range_contains_position(
            &data_target.source_range,
            position_for(content, "versions.toml")
        ));

        let target = frontmatter_document_target_at_position(
            &project,
            content,
            position_for(content, "custom.html"),
        )
        .expect("target lookup")
        .expect("target");
        assert_eq!(target.kind, FrontmatterDocumentKind::Template);
        assert_eq!(
            target.target_uri().expect("target uri"),
            Url::from_file_path(templates_dir.join("custom.html").as_std_path())
                .expect("expected uri")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn resolves_template_path_document_targets_from_authoring_model() {
        let dir = temp_dir("template-path-links");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        let source = "+++\ntitle = \"Uses Base\"\ntemplate = \"base.html\"\n+++\n\n# Uses Base\n";
        std::fs::write(content_dir.join("uses-base.md"), source).expect("write source");
        std::fs::write(
            templates_dir.join("base.html"),
            "{% block content %}{% endblock %}",
        )
        .expect("write base");
        std::fs::write(templates_dir.join("partial.html"), "<p>Partial</p>")
            .expect("write partial");
        std::fs::write(
            templates_dir.join("macros.html"),
            "{% macro card(title) %}{{ title }}{% endmacro %}",
        )
        .expect("write macros");

        let child = "{% extends \"base.html\" %}\n{% include \"partial.html\" %}\n{% import \"macros.html\" as macros %}\n";
        std::fs::write(templates_dir.join("child.html"), child).expect("write child");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let targets =
            template_document_targets(&project, "child.html", child).expect("template targets");
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].kind, TemplateDocumentKind::Extends);
        assert_eq!(targets[0].path, "base.html");
        assert_eq!(targets[0].target_path, templates_dir.join("base.html"));
        assert_eq!(targets[1].kind, TemplateDocumentKind::Include);
        assert_eq!(targets[1].path, "partial.html");
        assert_eq!(targets[1].target_path, templates_dir.join("partial.html"));
        assert_eq!(targets[2].kind, TemplateDocumentKind::Import);
        assert_eq!(targets[2].path, "macros.html");
        assert_eq!(targets[2].target_path, templates_dir.join("macros.html"));
        let references =
            template_document_references(&content_dir, &project, &targets[0].target_path)
                .expect("template document references");
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&dir)
                        .expect("project relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "content/uses-base.md".to_string(),
                "templates/child.html".to_string(),
            ]
        );

        let target = template_document_target_at_position(
            &project,
            "child.html",
            child,
            position_for(child, "partial.html"),
        )
        .expect("target lookup")
        .expect("target");
        assert_eq!(target.kind, TemplateDocumentKind::Include);
        assert_eq!(
            target.target_uri().expect("target uri"),
            Url::from_file_path(templates_dir.join("partial.html").as_std_path())
                .expect("expected uri")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn resolves_template_definition_targets_from_authoring_model() {
        let dir = temp_dir("template-definitions");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let base = "{% block content %}Base{% endblock %}\n";
        let macros = "{% macro card(title) %}{{ title }}{% endmacro %}\n";
        let child = "{% extends \"base.html\" %}\n{% import \"macros.html\" as macros %}\n{% block content %}{{ macros::card(\"Hi\") }} {{ title | trim }}{% if title is string %}ok{% endif %}{% endblock %}\n";

        std::fs::write(templates_dir.join("base.html"), base).expect("write base");
        std::fs::write(templates_dir.join("macros.html"), macros).expect("write macros");
        std::fs::write(templates_dir.join("child.html"), child).expect("write child");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let targets =
            template_definition_targets(&project, "child.html", child).expect("definition targets");

        let block = targets
            .iter()
            .find(|target| target.kind == TemplateDefinitionKind::Block && target.name == "content")
            .expect("block target");
        assert!(range_contains_position(
            &block.source_range,
            position_for(child, "content")
        ));
        assert_eq!(block.target_path, templates_dir.join("base.html"));
        assert!(range_contains_position(
            &block.target_range,
            position_for(base, "content")
        ));

        let macro_target = targets
            .iter()
            .find(|target| {
                target.kind == TemplateDefinitionKind::Macro && target.name == "macros::card"
            })
            .expect("macro target");
        assert!(range_contains_position(
            &macro_target.source_range,
            position_for(child, "card")
        ));
        assert_eq!(macro_target.target_path, templates_dir.join("macros.html"));
        assert!(range_contains_position(
            &macro_target.target_range,
            position_for(macros, "card")
        ));

        let filter = targets
            .iter()
            .find(|target| target.kind == TemplateDefinitionKind::Filter && target.name == "trim")
            .expect("filter target");
        assert!(range_contains_position(
            &filter.source_range,
            position_for(child, "trim")
        ));
        assert!(filter.target_path.ends_with("gingembre/src/eval.rs"));
        assert!(
            filter
                .hover_markdown()
                .contains("Removes leading and trailing whitespace")
        );

        let test = targets
            .iter()
            .find(|target| target.kind == TemplateDefinitionKind::Test && target.name == "string")
            .expect("test target");
        assert!(range_contains_position(
            &test.source_range,
            position_for(child, "string")
        ));
        assert!(test.target_path.ends_with("gingembre/src/eval.rs"));
        assert!(test.hover_markdown().contains("value is a string"));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn finds_imported_template_macro_references() {
        let dir = temp_dir("template-macro-references");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let macros = "{% macro card(title) %}{{ title }}{% endmacro %}\n";
        let page = "{% import \"macros.html\" as ui %}\n{{ ui::card(\"Hi\") }}\n";
        let other = "{% import \"macros.html\" as kit %}\n{% set rendered = kit::card(\"Yo\") %}\n";
        std::fs::write(templates_dir.join("macros.html"), macros).expect("write macros");
        std::fs::write(templates_dir.join("page.html"), page).expect("write page");
        std::fs::write(templates_dir.join("other.html"), other).expect("write other");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let index = TemplateAuthoringIndex::new(&project);
        let definition_query = index
            .macro_reference_query("macros.html", position_for(macros, "card"))
            .expect("definition query");
        assert_eq!(
            definition_query,
            TemplateMacroReferenceQuery {
                target_template_file: "macros.html".to_string(),
                macro_name: "card".to_string()
            }
        );

        let call_query = index
            .macro_reference_query("page.html", position_for(page, "card"))
            .expect("call query");
        assert_eq!(
            call_query,
            TemplateMacroReferenceQuery {
                target_template_file: "macros.html".to_string(),
                macro_name: "card".to_string()
            }
        );

        let references =
            template_macro_references(&index, "macros.html", "card").expect("references");
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&templates_dir)
                        .expect("template relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "macros.html".to_string(),
                "other.html".to_string(),
                "page.html".to_string(),
            ]
        );
        for (content, index) in [(macros, 0), (other, 0), (page, 0)] {
            assert!(references.iter().any(|location| range_contains_position(
                &location.range,
                position_for_nth(content, "card", index)
            )));
        }

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn resolves_template_block_references() {
        let dir = temp_dir("template-block-references");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let base = "{% block breadcrumbs %}Base{% endblock %}\n";
        let child = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
        let sibling = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Sibling{% endblock %}\n";
        std::fs::write(templates_dir.join("base.html"), base).expect("write base");
        std::fs::write(templates_dir.join("child.html"), child).expect("write child");
        std::fs::write(templates_dir.join("sibling.html"), sibling).expect("write sibling");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let index = TemplateAuthoringIndex::new(&project);
        let occurrence = index
            .block_occurrence_at_position("child.html", position_for(child, "breadcrumbs"))
            .expect("block occurrence");
        assert_eq!(occurrence.name, "breadcrumbs");

        let target = index
            .block_definition_target("child.html", &occurrence)
            .expect("parent block target");
        assert_eq!(target.target_path, templates_dir.join("base.html"));
        assert!(range_contains_position(
            &target.target_range,
            position_for(base, "breadcrumbs")
        ));

        let hover = template_block_hover_markdown(&index, "child.html", &occurrence);
        assert!(hover.contains("Overrides"));
        assert!(hover.contains("3 matching block declaration"));

        let references = template_block_references(&index, "child.html", "breadcrumbs")
            .expect("block references");
        assert_eq!(references.len(), 3);
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&templates_dir)
                        .expect("template relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "base.html".to_string(),
                "child.html".to_string(),
                "sibling.html".to_string(),
            ]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn resolves_template_block_references_by_inheritance_tree() {
        let dir = temp_dir("template-block-inheritance-references");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let base = "{% block breadcrumbs %}Base{% endblock %}\n";
        let child = "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
        let grandchild =
            "{% extends \"child.html\" %}\n{% block breadcrumbs %}Grandchild{% endblock %}\n";
        let other_base = "{% block breadcrumbs %}Other base{% endblock %}\n";
        let other_child =
            "{% extends \"other-base.html\" %}\n{% block breadcrumbs %}Other child{% endblock %}\n";
        std::fs::write(templates_dir.join("base.html"), base).expect("write base");
        std::fs::write(templates_dir.join("child.html"), child).expect("write child");
        std::fs::write(templates_dir.join("grandchild.html"), grandchild)
            .expect("write grandchild");
        std::fs::write(templates_dir.join("other-base.html"), other_base)
            .expect("write other base");
        std::fs::write(templates_dir.join("other-child.html"), other_child)
            .expect("write other child");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let index = TemplateAuthoringIndex::new(&project);

        let child_occurrence = index
            .block_occurrence_at_position("child.html", position_for(child, "breadcrumbs"))
            .expect("child block occurrence");
        let target = index
            .block_definition_target("child.html", &child_occurrence)
            .expect("child parent block");
        assert_eq!(target.target_path, templates_dir.join("base.html"));

        let references =
            template_block_references(&index, "child.html", "breadcrumbs").expect("references");
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&templates_dir)
                        .expect("template relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "base.html".to_string(),
                "child.html".to_string(),
                "grandchild.html".to_string(),
            ]
        );

        let other_index = TemplateAuthoringIndex::new(&project);
        let other_references =
            template_block_references(&other_index, "other-child.html", "breadcrumbs")
                .expect("other references");
        assert_eq!(
            other_references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&templates_dir)
                        .expect("template relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "other-base.html".to_string(),
                "other-child.html".to_string(),
            ]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn template_block_index_uses_current_template_content() {
        let dir = temp_dir("template-block-live-content");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let base = "{% block breadcrumbs %}Base{% endblock %}\n";
        let other_base = "{% block breadcrumbs %}Other base{% endblock %}\n";
        let disk_child =
            "{% extends \"base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
        let live_child =
            "{% extends \"other-base.html\" %}\n{% block breadcrumbs %}Child{% endblock %}\n";
        std::fs::write(templates_dir.join("base.html"), base).expect("write base");
        std::fs::write(templates_dir.join("other-base.html"), other_base)
            .expect("write other base");
        std::fs::write(templates_dir.join("child.html"), disk_child).expect("write child");

        let project = load_authoring_project(
            &content_dir,
            &[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Template("child.html".to_string()),
                content: live_child.to_string(),
            }],
        )
        .await
        .expect("load project");
        let index = TemplateAuthoringIndex::new(&project);
        let references =
            template_block_references(&index, "child.html", "breadcrumbs").expect("references");

        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&templates_dir)
                        .expect("template relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec!["child.html".to_string(), "other-base.html".to_string()]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn exposes_template_document_symbols() {
        let dir = temp_dir("template-symbols");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let template =
            "{% block content %}{% macro card(title) %}{{ title }}{% endmacro %}{% endblock %}\n";
        std::fs::write(templates_dir.join("page.html"), template).expect("write template");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let symbols =
            template_document_symbols(&project, "page.html", template).expect("document symbols");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "content");
        assert_eq!(symbols[0].kind, SymbolKind::MODULE);
        let children = symbols[0].children.as_ref().expect("block children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "card");
        assert_eq!(children[0].kind, SymbolKind::FUNCTION);

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn uses_template_semantic_index_for_editor_features() {
        let dir = temp_dir("template-semantic-editor-features");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let template = "{% set local_route = \"/\" %}\n{{ local_route }}\n{% for item in section.pages %}\n{{ item.path | path_parent }}\n{% endfor %}\n{% for child in section.subsections %}\n{{ child.pages }}\n{% endfor %}\n{% macro label(title) %}{{ title }}{% endmacro %}\n";
        std::fs::write(templates_dir.join("page.html"), template).expect("write template");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let index = project
            .template_semantics
            .get("page.html")
            .expect("template semantics");

        let hover = template_semantic_hover(
            &project,
            "page.html",
            template,
            position_for(template, "path"),
        )
        .expect("field hover");
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markdown hover");
        };
        assert!(markup.value.contains("Route path"));
        assert!(markup.value.contains("Site-relative route"));

        let filter_hover = template_semantic_hover(
            &project,
            "page.html",
            template,
            position_for(template, "path_parent"),
        )
        .expect("filter hover");
        let HoverContents::Markup(markup) = filter_hover.contents else {
            panic!("expected filter markdown hover");
        };
        assert!(markup.value.contains("Gingembre filter"));
        assert!(markup.value.contains("Returns the parent path"));

        let section_field_hover = template_semantic_hover(
            &project,
            "page.html",
            template,
            position_for_nth(template, "pages", 1),
        )
        .expect("section field hover through loop binding");
        let HoverContents::Markup(markup) = section_field_hover.contents else {
            panic!("expected section field markdown hover");
        };
        assert!(markup.value.contains("Section pages"));
        assert!(markup.value.contains("nearest parent section"));

        let definition = template_semantic_definition(
            &content_dir,
            &project,
            "page.html",
            position_for_nth(template, "local_route", 1),
        )
        .expect("local binding definition");
        assert!(range_contains_position(
            &definition.range,
            position_for(template, "local_route")
        ));

        let references = template_semantic_references(
            &content_dir,
            &project,
            "page.html",
            template,
            position_for_nth(template, "local_route", 1),
        );
        assert_eq!(references.len(), 2);
        assert!(references.iter().any(|location| range_contains_position(
            &location.range,
            position_for(template, "local_route")
        )));
        assert!(references.iter().any(|location| range_contains_position(
            &location.range,
            position_for_nth(template, "local_route", 1)
        )));

        let prepared = template_semantic_prepare_rename(
            &project,
            "page.html",
            template,
            position_for_nth(template, "local_route", 1),
        )
        .expect("prepare rename");
        let PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } = prepared else {
            panic!("expected range with placeholder");
        };
        assert_eq!(placeholder, "local_route");

        let uri = Url::from_file_path(templates_dir.join("page.html").as_std_path())
            .expect("template uri");
        let edit = template_semantic_rename_workspace_edit(
            &uri,
            &project,
            "page.html",
            template,
            position_for_nth(template, "local_route", 1),
            "route_path",
        )
        .expect("rename edit")
        .expect("rename edit");
        let mut changes = edit.changes.expect("rename changes");
        let edits = changes.remove(&uri).expect("template edits");
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|edit| edit.new_text == "route_path"));

        let token_types = template_semantic_tokens(index, template)
            .into_iter()
            .map(|token| token.token_type)
            .collect::<Vec<_>>();
        assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_VARIABLE));
        assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_PARAMETER));
        assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_PROPERTY));
        assert!(token_types.contains(&TEMPLATE_SEMANTIC_TOKEN_MACRO));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn repeated_template_set_bindings_share_references() {
        let dir = temp_dir("template-repeated-set-references");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let template = "{% set current_path = \"/\" %}\n{% if section is defined %}{% set current_path = section.path %}{% endif %}\n{% if page is defined %}{% set current_path = page.path %}{% endif %}\n<a class=\"{% if current_path is eq(\"/\") %}is-active{% endif %}\">{{ current_path }}</a>\n";
        std::fs::write(templates_dir.join("base.html"), template).expect("write template");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let first_definition = template_semantic_definition(
            &content_dir,
            &project,
            "base.html",
            position_for(template, "current_path"),
        )
        .expect("first definition");
        let second_definition = template_semantic_definition(
            &content_dir,
            &project,
            "base.html",
            position_for_nth(template, "current_path", 1),
        )
        .expect("second definition");
        assert_eq!(first_definition.range, second_definition.range);
        assert!(range_contains_position(
            &first_definition.range,
            position_for(template, "current_path")
        ));

        let first_references = template_semantic_references(
            &content_dir,
            &project,
            "base.html",
            template,
            position_for(template, "current_path"),
        );
        let second_references = template_semantic_references(
            &content_dir,
            &project,
            "base.html",
            template,
            position_for_nth(template, "current_path", 1),
        );
        assert_eq!(first_references, second_references);
        assert_eq!(first_references.len(), 5);
        for index in 0..5 {
            assert!(
                first_references
                    .iter()
                    .any(|location| range_contains_position(
                        &location.range,
                        position_for_nth(template, "current_path", index)
                    ))
            );
        }

        let uri =
            Url::from_file_path(templates_dir.join("base.html").as_std_path()).expect("file uri");
        let edit = template_semantic_rename_workspace_edit(
            &uri,
            &project,
            "base.html",
            template,
            position_for(template, "current_path"),
            "active_path",
        )
        .expect("rename edit")
        .expect("rename edit");
        let edits = edit
            .changes
            .expect("changes")
            .remove(&uri)
            .expect("template edits");
        assert_eq!(edits.len(), 5);

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn reports_template_diagnostics_from_authoring_model() {
        let dir = temp_dir("template-diagnostics");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        std::fs::write(
            templates_dir.join("base.html"),
            "{% block title %}Title{% endblock %}\n",
        )
        .expect("write base");
        std::fs::write(
            templates_dir.join("macros.html"),
            "{% macro card(title) %}{{ title }}{% endmacro %}\n",
        )
        .expect("write macros");
        let child = "{% extends \"base.html\" %}\n{% include \"missing.html\" %}\n{% import \"macros.html\" as macros %}\n{% block content %}{{ macros::missing(\"Hi\") | nope }}{% if title is frobnicate %}ok{% endif %}{% endblock %}\n";
        std::fs::write(templates_dir.join("child.html"), child).expect("write child");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let diagnostics = diagnostics_for_template(&project, "child.html", child);
        let kinds = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect::<Vec<_>>();

        assert!(kinds.contains(&AuthoringDiagnosticKind::MissingTemplate));
        assert!(kinds.contains(&AuthoringDiagnosticKind::MissingBlock));
        assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownMacro));
        assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownFilter));
        assert!(kinds.contains(&AuthoringDiagnosticKind::UnknownTest));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AuthoringDiagnosticKind::MissingTemplate
                && diagnostic.target == "missing.html"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AuthoringDiagnosticKind::MissingBlock
                && diagnostic.target == "content"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AuthoringDiagnosticKind::UnknownMacro
                && diagnostic.target == "missing"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AuthoringDiagnosticKind::UnknownFilter && diagnostic.target == "nope"
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == AuthoringDiagnosticKind::UnknownTest
                && diagnostic.target == "frobnicate"
        }));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn completes_template_context_filters_tests_data_and_macros() {
        let dir = temp_dir("template-completions");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(data_dir.join("versions.toml"), "dodeca = \"0.1.0\"\n").expect("write data");
        std::fs::write(
            templates_dir.join("macros.html"),
            "{% macro card(title) %}{{ title }}{% endmacro %}\n",
        )
        .expect("write macros");

        let template = "{% import \"macros.html\" as macros %}\n{% set local_route = \"/\" %}\n{% set current_page = page %}\n{{ loc }}\n{{ pa }}\n{{ page.ti }}\n{{ current_page.pa }}\n{% for item in section.pages %}\n{{ item.pa }}\n{% endfor %}\n{{ data.ver }}\n{{ title | tr }}\n{% if title is str %}ok{% endif %}\n{{ macros::card() }}\n";
        std::fs::write(templates_dir.join("page.html"), template).expect("write template");
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");

        let labels = |position| {
            template_completion_items(&project, "page.html", template, position)
                .into_iter()
                .map(|item| item.label)
                .collect::<Vec<_>>()
        };

        let root_items = template_completion_items(
            &project,
            "page.html",
            template,
            position_for(template, "pa"),
        );
        let page_item = root_items
            .iter()
            .find(|item| item.label == "page")
            .expect("page completion item");
        assert_eq!(page_item.detail.as_deref(), Some("Current page"));
        let Some(Documentation::MarkupContent(documentation)) = &page_item.documentation else {
            panic!("expected page completion docs");
        };
        assert!(documentation.value.contains("currently being rendered"));

        assert!(
            labels(position_for(template, "loc")).contains(&"local_route".to_string()),
            "root completion should include live Gingembre local symbols"
        );
        assert!(labels(position_for(template, "pa")).contains(&"page".to_string()));
        assert!(
            labels(position_for(template, "ti")).contains(&"title".to_string()),
            "page field completion should include title"
        );
        let current_page_field_byte = template
            .find("current_page.pa")
            .expect("current page alias field")
            + "current_page.".len();
        let (line, column) = byte_to_line_column(template, current_page_field_byte);
        assert!(
            labels(Position::new(line - 1, column - 1)).contains(&"path".to_string()),
            "set aliases should complete fields from their source value"
        );
        let loop_item_field_byte =
            template.find("item.pa").expect("loop item field") + "item.".len();
        let (line, column) = byte_to_line_column(template, loop_item_field_byte);
        assert!(
            labels(Position::new(line - 1, column - 1)).contains(&"permalink".to_string()),
            "loop bindings over section.pages should complete page fields"
        );
        assert!(
            labels(position_for(template, "ver")).contains(&"versions".to_string()),
            "data completion should include data file stem"
        );
        assert!(
            labels(position_for(template, "tr")).contains(&"trim".to_string()),
            "filter completion should include trim"
        );
        let filter_items = template_completion_items(
            &project,
            "page.html",
            template,
            position_for(template, "tr"),
        );
        let trim_item = filter_items
            .iter()
            .find(|item| item.label == "trim")
            .expect("trim completion item");
        let Some(Documentation::MarkupContent(documentation)) = &trim_item.documentation else {
            panic!("expected trim completion docs");
        };
        assert!(
            documentation
                .value
                .contains("Removes leading and trailing whitespace")
        );
        assert!(
            labels(position_for(template, "str")).contains(&"string".to_string()),
            "test completion should include string"
        );
        let macro_byte = template.find("macros::").expect("macro call") + "macros::".len();
        let (line, column) = byte_to_line_column(template, macro_byte);
        assert!(
            labels(Position::new(line - 1, column - 1)).contains(&"card".to_string()),
            "macro completion should include imported macro"
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn resolves_authoring_dirs_from_lsp_workspace_folder() {
        let dir = temp_dir("initialize-root");
        std::fs::create_dir_all(dir.join(".config")).expect("create config dir");
        std::fs::create_dir_all(dir.join("content")).expect("create content dir");
        std::fs::write(
            dir.join(".config/dodeca.styx"),
            "content content\noutput public\n",
        )
        .expect("write config");

        let params = initialize_params_for_workspace(&dir);
        let dirs = resolve_initial_authoring_dirs(&default_startup_args(), &params)
            .expect("resolve dirs")
            .expect("workspace config");

        assert_eq!(dirs.content_dir, dir.join("content"));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn initializes_without_lsp_workspace() {
        let dirs =
            resolve_initial_authoring_dirs(&default_startup_args(), &empty_initialize_params())
                .expect("initialize without workspace");

        assert!(dirs.is_none());
    }

    #[test]
    fn resolves_authoring_dirs_from_lsp_workspace_descendant() {
        let project = temp_dir("initialize-descendant");
        let content_dir = project.join("content");
        std::fs::create_dir_all(project.join(".config")).expect("create config dir");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::write(
            project.join(".config/dodeca.styx"),
            "content content\noutput public\n",
        )
        .expect("write config");

        let params = initialize_params_for_workspace(&content_dir);
        let dirs = resolve_initial_authoring_dirs(&default_startup_args(), &params)
            .expect("resolve dirs")
            .expect("workspace config");

        assert_eq!(dirs.content_dir, content_dir);

        std::fs::remove_dir_all(&project).expect("remove temp dir");
    }

    #[test]
    fn resolves_authoring_dirs_from_document_ancestor_config() {
        let workspace = temp_dir("document-root");
        let project = workspace.join("kb.vixen.rs");
        std::fs::create_dir_all(project.join(".config")).expect("create config dir");
        std::fs::create_dir_all(project.join("content/ops")).expect("create content dir");
        std::fs::write(
            project.join(".config/dodeca.styx"),
            "content content\noutput public\n",
        )
        .expect("write config");
        let document = project.join("content/ops/deploy.md");
        std::fs::write(&document, "# Deploy\n").expect("write document");
        let uri = Url::from_file_path(document.as_std_path()).expect("document uri");

        let dirs = resolve_authoring_dirs_for_document(&default_startup_args(), &uri)
            .expect("resolve document dirs");

        assert_eq!(dirs.content_dir, project.join("content"));

        std::fs::remove_dir_all(&workspace).expect("remove temp dir");
    }

    #[tokio::test]
    async fn lists_pages_and_sections_from_content_dir() {
        let dir = temp_dir("list-pages");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create dirs");
        std::fs::write(
            content_dir.join("_index.md"),
            "+++\ntitle = \"Knowledge Base\"\n+++\n\n# Home\n",
        )
        .expect("write root");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n## Details\n",
        )
        .expect("write page");

        let pages = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project")
            .pages;

        assert_eq!(
            pages,
            vec![
                AuthoringPage {
                    kind: AuthoringPageKind::Section,
                    route: "/".to_string(),
                    source_file: "_index.md".to_string(),
                    title: "Knowledge Base".to_string(),
                    description: None,
                    template: "index.html".to_string(),
                    output_path: "index.html".to_string(),
                    headings: vec![crate::authoring_model::AuthoringHeading {
                        id: "home".to_string(),
                        title: "Home".to_string(),
                        level: 1,
                    }],
                    heading_ids: vec!["home".to_string()],
                    link_base_route: "/".to_string(),
                },
                AuthoringPage {
                    kind: AuthoringPageKind::Page,
                    route: "/guide/intro".to_string(),
                    source_file: "guide/intro.md".to_string(),
                    title: "Intro".to_string(),
                    description: None,
                    template: "page.html".to_string(),
                    output_path: "guide/intro/index.html".to_string(),
                    headings: vec![
                        crate::authoring_model::AuthoringHeading {
                            id: "intro".to_string(),
                            title: "Intro".to_string(),
                            level: 1,
                        },
                        crate::authoring_model::AuthoringHeading {
                            id: "intro--details".to_string(),
                            title: "Details".to_string(),
                            level: 2,
                        },
                    ],
                    heading_ids: vec!["intro".to_string(), "intro--details".to_string()],
                    link_base_route: "/".to_string(),
                },
            ]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn reports_missing_routes_anchors_sources_and_static_assets() {
        let dir = temp_dir("diagnostics");
        let content_dir = dir.join("content");
        let static_dir = dir.join("static");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::create_dir_all(&static_dir).expect("create static dir");
        std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(content_dir.join("guide/intro.md"), "# Intro\n").expect("write target");
        let source = "\
# Source

[ok](/guide/intro#intro)
[missing route](/missing)
[missing anchor](/guide/intro#nope)
[missing source](@/guide/missing.md)
![missing image](/missing.png)
![ok image](/logo.png)
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let source_page = project
            .page_for_source_file("guide/source.md")
            .expect("source page");
        let diagnostics = diagnostics_for_page(&project, source_page, source);
        let kinds = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind)
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                AuthoringDiagnosticKind::Route,
                AuthoringDiagnosticKind::Anchor,
                AuthoringDiagnosticKind::Source,
                AuthoringDiagnosticKind::StaticAsset,
            ]
        );
        assert_eq!(diagnostics[0].source_file, "guide/source.md");
        assert_eq!(diagnostics[0].resolved_route.as_deref(), Some("/missing"));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn reports_site_graph_diagnostics() {
        let dir = temp_dir("site-graph-diagnostics");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(content_dir.join("empty")).expect("create empty section");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(
            templates_dir.join("section.html"),
            "<nav><a href=\"/rendered\">Rendered</a></nav>{{ section.content | safe }}",
        )
        .expect("write section template");
        std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
            .expect("write page template");
        std::fs::write(
            content_dir.join("_index.md"),
            "+++\ntitle = \"Home\"\ntemplate = \"section.html\"\n+++\n\n[linked](/linked)\n[same a](/same-a)\n[same b](/same-b)\n",
        )
        .expect("write root");
        std::fs::write(
            content_dir.join("linked.md"),
            "+++\ntitle = \"Linked\"\n+++\n",
        )
        .expect("write linked");
        std::fs::write(
            content_dir.join("orphan.md"),
            "+++\ntitle = \"Orphan\"\n+++\n",
        )
        .expect("write orphan");
        std::fs::write(
            content_dir.join("rendered.md"),
            "+++\ntitle = \"Rendered\"\n+++\n",
        )
        .expect("write rendered-nav page");
        std::fs::write(
            content_dir.join("same-a.md"),
            "+++\ntitle = \"Same\"\n+++\n",
        )
        .expect("write same a");
        std::fs::write(
            content_dir.join("same-b.md"),
            "+++\ntitle = \"Same\"\n+++\n",
        )
        .expect("write same b");
        std::fs::write(
            content_dir.join("empty/_index.md"),
            "+++\ntitle = \"Empty\"\n+++\n",
        )
        .expect("write empty section");

        let mut project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        assert!(
            project
                .rendered_hrefs_by_route
                .get("/")
                .is_some_and(|hrefs| hrefs.iter().any(|href| href.href == "/rendered"))
        );
        let duplicate_route_source = project
            .pages
            .iter()
            .find(|page| page.source_file == "same-b.md")
            .map(|page| page.route.clone())
            .expect("same-b route");
        for page in &mut project.pages {
            if page.source_file == "same-b.md" {
                page.route = "/same-a".to_string();
            }
        }

        let diagnostics = site_graph_diagnostics(&project);
        let has = |kind, target: &str| {
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == kind && diagnostic.target == target)
        };

        assert!(has(AuthoringDiagnosticKind::DuplicateTitle, "Same"));
        assert!(has(AuthoringDiagnosticKind::DuplicateRoute, "/same-a"));
        assert!(has(AuthoringDiagnosticKind::OrphanPage, "/orphan"));
        assert!(
            has(AuthoringDiagnosticKind::NoInboundLinks, "/empty/")
                || has(AuthoringDiagnosticKind::NoInboundLinks, "/empty")
        );
        assert!(!has(AuthoringDiagnosticKind::OrphanPage, "/linked"));
        assert!(!has(AuthoringDiagnosticKind::OrphanPage, "/rendered"));
        assert_ne!(duplicate_route_source, "/same-a");

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn exposes_route_graph_edges() {
        let dir = temp_dir("route-graph");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(
            templates_dir.join("section.html"),
            "<nav><a href=\"/rendered\">Rendered</a></nav>{{ section.content | safe }}",
        )
        .expect("write section template");
        std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
            .expect("write page template");
        std::fs::write(
            content_dir.join("_index.md"),
            "+++\ntitle = \"Home\"\ntemplate = \"section.html\"\n+++\n\n[guide](/guide)\n",
        )
        .expect("write root");
        std::fs::write(
            content_dir.join("guide.md"),
            "+++\ntitle = \"Guide\"\n+++\n\n[home](/)\n",
        )
        .expect("write guide");
        std::fs::write(
            content_dir.join("rendered.md"),
            "+++\ntitle = \"Rendered\"\n+++\n",
        )
        .expect("write rendered");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let graph = route_graph_for_project(&project);
        let home = graph
            .iter()
            .find(|node| node.route == "/")
            .expect("home node");
        let guide = graph
            .iter()
            .find(|node| node.route == "/guide")
            .expect("guide node");
        let rendered = graph
            .iter()
            .find(|node| node.route == "/rendered")
            .expect("rendered node");

        assert!(home.outgoing.iter().any(|edge| {
            edge.kind == RouteGraphEdgeKind::Markdown && edge.target_route == "/guide"
        }));
        assert!(home.outgoing.iter().any(|edge| {
            edge.kind == RouteGraphEdgeKind::RenderedHtml && edge.target_route == "/rendered"
        }));
        assert_eq!(guide.incoming.len(), 1);
        assert_eq!(guide.incoming[0].source_route, "/");
        assert_eq!(guide.outgoing.len(), 1);
        assert_eq!(guide.outgoing[0].target_route, "/");
        assert_eq!(rendered.incoming.len(), 1);
        assert_eq!(rendered.incoming[0].kind, RouteGraphEdgeKind::RenderedHtml);

        let json = route_graph_to_json(&graph);
        assert!(json.as_array().is_some_and(|nodes| nodes.len() == 3));
        assert!(
            json.to_string().contains("\"kind\":\"renderedHtml\""),
            "route graph JSON should preserve rendered edge provenance"
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn code_actions_fix_missing_anchors_from_authoring_headings() {
        let dir = temp_dir("anchor-actions");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        let target = "\
# Intro

## Details
";
        std::fs::write(content_dir.join("guide/intro.md"), target).expect("write target");
        let source = "\
# Source

[typo](/guide/intro#intro--detals)
[new](/guide/intro#intro--appendix)
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let source_page = project
            .page_for_source_file("guide/source.md")
            .expect("source page");
        let diagnostics = diagnostics_for_page(&project, source_page, source);
        let lsp_diagnostics = diagnostics
            .iter()
            .map(authoring_diagnostic_to_lsp)
            .collect::<Vec<_>>();
        let typo_diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.target.contains("intro--detals"))
            .expect("typo diagnostic");
        let actions = missing_anchor_code_actions(
            &content_dir,
            &project,
            source,
            typo_diagnostic,
            &lsp_diagnostics,
        );

        let titles = actions
            .iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action) => Some(action.title.as_str()),
                CodeActionOrCommand::Command(_) => None,
            })
            .collect::<Vec<_>>();
        assert!(titles.contains(&"Change anchor to '#intro--details'"));
        assert!(titles.contains(&"Create heading for '#intro--detals'"));

        let source_uri =
            Url::from_file_path(content_dir.join("guide/source.md")).expect("source uri");
        let replacement = actions
            .iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Change anchor to '#intro--details'" =>
                {
                    action.edit.as_ref()
                }
                _ => None,
            })
            .next()
            .expect("replacement edit");
        let replacement_edits = replacement
            .changes
            .as_ref()
            .and_then(|changes| changes.get(&source_uri))
            .expect("source replacement edits");
        assert_eq!(replacement_edits.len(), 1);
        assert_eq!(replacement_edits[0].new_text, "intro--details");

        let target_uri =
            Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri");
        let creation = actions
            .iter()
            .filter_map(|action| match action {
                CodeActionOrCommand::CodeAction(action)
                    if action.title == "Create heading for '#intro--detals'" =>
                {
                    action.edit.as_ref()
                }
                _ => None,
            })
            .next()
            .expect("creation edit");
        let creation_edits = creation
            .changes
            .as_ref()
            .and_then(|changes| changes.get(&target_uri))
            .expect("target creation edits");
        assert_eq!(creation_edits.len(), 1);
        assert_eq!(creation_edits[0].new_text, "\n\n## Detals\n");

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn code_action_extracts_selection_to_page_without_duplicate_title() {
        let dir = temp_dir("extract-page");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write guide");
        let source = "\
+++
title = \"Source\"
+++

# Source

Keep this.

## Extract Me

This moves.
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let uri = Url::from_file_path(content_dir.join("guide/source.md")).expect("source uri");
        let selection = range_for(source, "## Extract Me\n\nThis moves.");
        let plan = extract_page_plan(&content_dir, &project, &uri, source, selection)
            .expect("extract plan")
            .expect("extract action");

        assert_eq!(plan.source_file, "guide/source.md");
        assert_eq!(plan.new_source_file, "guide/extract-me.md");
        assert_eq!(plan.new_route, "/guide/extract-me");
        assert_eq!(plan.title, "Extract Me");
        assert_eq!(
            plan.new_content,
            "+++\ntitle = \"Extract Me\"\n+++\n\nThis moves.\n"
        );
        assert_eq!(plan.replacement, "[Extract Me](extract-me)");
        assert!(!plan.new_content.contains("## Extract Me"));

        let edit = workspace_edit_for_extract_page(&content_dir, &plan).expect("workspace edit");
        let operations = match edit.document_changes.expect("document changes") {
            DocumentChanges::Operations(operations) => operations,
            DocumentChanges::Edits(_) => panic!("expected document change operations"),
        };
        assert_eq!(operations.len(), 3);
        match &operations[0] {
            DocumentChangeOperation::Op(ResourceOp::Create(create)) => {
                assert_eq!(
                    create.uri,
                    Url::from_file_path(content_dir.join("guide/extract-me.md"))
                        .expect("created uri")
                );
            }
            _ => panic!("expected create file operation"),
        }

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn resolves_definition_locations_for_links_and_static_assets() {
        let dir = temp_dir("definition");
        let content_dir = dir.join("content");
        let static_dir = dir.join("static");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::create_dir_all(&static_dir).expect("create static dir");
        std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "# Intro\n\n## Details\n",
        )
        .expect("write target");
        std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write section");
        let source = "\
# Source

[route](/guide/intro#intro--details)
[source](@/guide/intro.md#intro)
[relative](intro#intro)
![logo](/logo.png)
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let dirs = AuthoringDirs {
            content_dir: content_dir.clone(),
        };
        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let page = project
            .page_for_source_file("guide/source.md")
            .expect("source page");

        let route_reference =
            reference_at_position(source, position_for(source, "/guide/intro#intro--details"))
                .expect("route reference");
        let route_location = definition_for_reference(&dirs, &project, page, &route_reference)
            .expect("route definition")
            .expect("route location");
        assert_eq!(
            route_location.uri,
            Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri")
        );
        assert_eq!(route_location.range.start.line, 2);

        let source_reference =
            reference_at_position(source, position_for(source, "@/guide/intro.md#intro"))
                .expect("source reference");
        let source_location = definition_for_reference(&dirs, &project, page, &source_reference)
            .expect("source definition")
            .expect("source location");
        assert_eq!(source_location.range.start.line, 0);

        let relative_reference = reference_at_position(source, position_for(source, "[relative]"))
            .expect("relative reference");
        let relative_location =
            definition_for_reference(&dirs, &project, page, &relative_reference)
                .expect("relative definition")
                .expect("relative location");
        assert_eq!(
            relative_location.uri,
            Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri")
        );

        let static_reference = reference_at_position(source, position_for(source, "/logo.png"))
            .expect("image reference");
        let static_location = definition_for_reference(&dirs, &project, page, &static_reference)
            .expect("static definition")
            .expect("static location");
        assert_eq!(
            static_location.uri,
            Url::from_file_path(static_dir.join("logo.png")).expect("static uri")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn finds_references_to_page_routes_and_source_links() {
        let dir = temp_dir("references");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        let section_template =
            "<nav><a href=\"/target\">Target</a></nav>{{ section.content | safe }}";
        std::fs::write(templates_dir.join("section.html"), section_template)
            .expect("write section template");
        std::fs::write(templates_dir.join("page.html"), "{{ page.content | safe }}")
            .expect("write page template");
        std::fs::write(
            content_dir.join("target.md"),
            "+++\ntitle = \"Target\"\n+++\n\n# Target\n",
        )
        .expect("write target");
        std::fs::write(
            content_dir.join("_index.md"),
            "\
+++
title = \"Home\"
template = \"section.html\"
+++

# Home

[route](/target)
[source](@/target.md)
[anchor](/target#target)
![asset](/target.png)
",
        )
        .expect("write root");
        std::fs::write(
            content_dir.join("nested/source.md"),
            "\
# Source

[relative](../target)
",
        )
        .expect("write nested source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let target_page = project
            .page_for_source_file("target.md")
            .expect("target page");
        let template_reference = template_route_reference_at_position(
            &project,
            "section.html",
            section_template,
            position_for(section_template, "/target"),
        )
        .expect("template route reference");
        assert_eq!(template_reference.target_route, "/target");
        assert!(range_contains_position(
            &template_reference.source_range,
            position_for(section_template, "/target")
        ));

        let references =
            references_to_page(&content_dir, &project, target_page).expect("references");

        assert_eq!(references.len(), 5);
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    let path =
                        Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                            .expect("utf8 path");
                    path.strip_prefix(&dir)
                        .expect("project relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "content/_index.md".to_string(),
                "content/_index.md".to_string(),
                "content/_index.md".to_string(),
                "content/nested/source.md".to_string(),
                "templates/section.html".to_string(),
            ]
        );
        let template_reference = references
            .iter()
            .find(|location| {
                location.uri.to_file_path().is_ok_and(|path| {
                    Utf8PathBuf::from_path_buf(path)
                        .ok()
                        .is_some_and(|path| path.ends_with("templates/section.html"))
                })
            })
            .expect("template rendered reference");
        assert!(range_contains_position(
            &template_reference.range,
            position_for(section_template, "/target")
        ));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn finds_references_to_exact_heading_fragments() {
        let dir = temp_dir("heading-references");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
        let target = "\
+++
title = \"Target\"
+++

# Target

## Details
";
        std::fs::write(content_dir.join("target.md"), target).expect("write target");
        std::fs::write(
            content_dir.join("_index.md"),
            "\
# Home

[page](/target)
[route heading](/target#target--details)
[source heading](@/target.md#target--details)
[wrong heading](/target#target)
",
        )
        .expect("write root");
        std::fs::write(
            content_dir.join("nested/source.md"),
            "\
# Source

[relative heading](../target#target--details)
",
        )
        .expect("write nested source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let target_page = project
            .page_for_source_file("target.md")
            .expect("target page");
        let heading_id =
            heading_id_at_position(target_page, target, position_for(target, "Details"))
                .expect("heading id");
        let references = references_to_heading(&content_dir, &project, target_page, &heading_id)
            .expect("heading references");

        assert_eq!(heading_id, "target--details");
        assert_eq!(references.len(), 3);
        assert_eq!(
            references
                .iter()
                .map(|location| {
                    Utf8PathBuf::from_path_buf(location.uri.to_file_path().expect("file uri"))
                        .expect("utf8 path")
                        .strip_prefix(&content_dir)
                        .expect("content relative")
                        .to_string()
                })
                .collect::<Vec<_>>(),
            vec![
                "_index.md".to_string(),
                "_index.md".to_string(),
                "nested/source.md".to_string(),
            ]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn renames_heading_and_exact_fragment_links() {
        let dir = temp_dir("heading-rename");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
        let target = "\
+++
title = \"Target\"
+++

# Target

## Details
";
        std::fs::write(content_dir.join("target.md"), target).expect("write target");
        let index = "\
# Home

[page](/target)
[route heading](/target#target--details)
[source heading](@/target.md#target--details)
[wrong heading](/target#target)
";
        std::fs::write(content_dir.join("_index.md"), index).expect("write root");
        let nested = "\
# Source

[relative heading](../target#target--details)
";
        std::fs::write(content_dir.join("nested/source.md"), nested).expect("write nested source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let target_page = project
            .page_for_source_file("target.md")
            .expect("target page");
        let target_position = position_for(target, "Details");
        let target = heading_rename_target_at_position(target_page, target, target_position)
            .expect("rename target");
        assert_eq!(target.heading_id, "target--details");
        assert_eq!(target.title, "Details");

        let edit = rename_heading_workspace_edit(
            &content_dir,
            &project,
            target_page,
            "target.md",
            project
                .source_contents
                .get("target.md")
                .expect("target content"),
            &target,
            "Deep Details",
        )
        .expect("rename edit")
        .expect("workspace edit");
        let changes = edit.changes.expect("workspace changes");

        let target_uri = Url::from_file_path(content_dir.join("target.md")).expect("target uri");
        let index_uri = Url::from_file_path(content_dir.join("_index.md")).expect("index uri");
        let nested_uri =
            Url::from_file_path(content_dir.join("nested/source.md")).expect("nested uri");

        let target_edits = changes.get(&target_uri).expect("target edits");
        assert_eq!(target_edits.len(), 1);
        assert_eq!(target_edits[0].new_text, "Deep Details");
        assert_eq!(target_edits[0].range, target.title_range);

        let index_edits = changes.get(&index_uri).expect("index edits");
        assert_eq!(
            index_edits
                .iter()
                .map(|edit| edit.new_text.as_str())
                .collect::<Vec<_>>(),
            vec!["target--deep-details", "target--deep-details"]
        );

        let nested_edits = changes.get(&nested_uri).expect("nested edits");
        assert_eq!(nested_edits.len(), 1);
        assert_eq!(nested_edits[0].new_text, "target--deep-details");
        assert_eq!(changes.len(), 3);

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn renames_page_route_source_file_and_resolved_links() {
        let dir = temp_dir("page-route-rename");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create guide dir");
        std::fs::create_dir_all(content_dir.join("nested")).expect("create nested dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write guide");
        std::fs::write(content_dir.join("nested/_index.md"), "# Nested\n").expect("write nested");
        std::fs::write(
            templates_dir.join("page.html"),
            "<nav><a href=\"/guide/intro#intro\">Intro</a><a href=\"/elsewhere\">Elsewhere</a></nav>{{ page.content | safe }}",
        )
        .expect("write page template");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "\
+++
title = \"Intro\"
+++

# Intro

[local](#intro)
[absolute self](/guide/intro#intro)
[source self](@/guide/intro.md#intro)
",
        )
        .expect("write target");
        std::fs::write(
            content_dir.join("guide/source.md"),
            "\
# Source

[relative route](intro#intro)
[relative source](intro.md#intro)
[wrong](/guide/other)
",
        )
        .expect("write guide source");
        std::fs::write(
            content_dir.join("nested/source.md"),
            "\
# Source

[absolute](/guide/intro#intro)
[absolute source](@/guide/intro.md#intro)
[relative route](../guide/intro#intro)
[relative source](../guide/intro.md#intro)
",
        )
        .expect("write nested source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let target_page = project
            .page_for_source_file("guide/intro.md")
            .expect("target page");
        let plan = page_route_rename_plan(&content_dir, &project, target_page, "/manual/setup")
            .expect("rename plan")
            .expect("page route rename");

        assert_eq!(plan.old_route, "/guide/intro");
        assert_eq!(plan.new_route, "/manual/setup");
        assert_eq!(plan.old_source_file, "guide/intro.md");
        assert_eq!(plan.new_source_file, "manual/setup.md");
        assert_eq!(
            plan.text_edits
                .iter()
                .map(|edit| (
                    page_route_text_edit_sort_key(&edit.path),
                    edit.new_target.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "source:guide/source.md".to_string(),
                    "../manual/setup#intro"
                ),
                (
                    "source:guide/source.md".to_string(),
                    "../manual/setup.md#intro"
                ),
                ("source:manual/setup.md".to_string(), "/manual/setup#intro"),
                (
                    "source:manual/setup.md".to_string(),
                    "@/manual/setup.md#intro"
                ),
                ("source:nested/source.md".to_string(), "/manual/setup#intro"),
                (
                    "source:nested/source.md".to_string(),
                    "@/manual/setup.md#intro"
                ),
                (
                    "source:nested/source.md".to_string(),
                    "../manual/setup#intro"
                ),
                (
                    "source:nested/source.md".to_string(),
                    "../manual/setup.md#intro"
                ),
                ("template:page.html".to_string(), "/manual/setup#intro"),
            ]
        );

        let workspace_edit =
            workspace_edit_for_page_route_rename(&content_dir, &plan).expect("workspace edit");
        let operations = match workspace_edit.document_changes.expect("document changes") {
            DocumentChanges::Operations(operations) => operations,
            DocumentChanges::Edits(_) => panic!("expected document change operations"),
        };
        match &operations[0] {
            DocumentChangeOperation::Op(ResourceOp::Rename(rename)) => {
                assert_eq!(
                    rename.old_uri,
                    Url::from_file_path(content_dir.join("guide/intro.md")).expect("old uri")
                );
                assert_eq!(
                    rename.new_uri,
                    Url::from_file_path(content_dir.join("manual/setup.md")).expect("new uri")
                );
            }
            _ => panic!("expected file rename operation"),
        }
        assert_eq!(operations.len(), 5);

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn completes_routes_sources_static_assets_and_headings_from_authoring_project() {
        let dir = temp_dir("completions");
        let content_dir = dir.join("content");
        let static_dir = dir.join("static");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::create_dir_all(&static_dir).expect("create static dir");
        std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write section");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "# Intro\n\n## Details\n",
        )
        .expect("write intro");
        let source = "\
# Source

[absolute](/)
[source](@/)
[relative]()
[heading](/guide/intro#)
[local](#)
![image](/)
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let page = project
            .page_for_source_file("guide/source.md")
            .expect("source page");
        let contexts = markdown_target_contexts(source);

        assert!(
            completion_new_texts(&project, page, &contexts[0]).contains(&"/guide/intro".into())
        );
        assert!(
            completion_new_texts(&project, page, &contexts[1]).contains(&"@/guide/intro.md".into())
        );
        assert!(completion_new_texts(&project, page, &contexts[2]).contains(&"intro".into()));
        assert!(
            completion_new_texts(&project, page, &contexts[3])
                .contains(&"/guide/intro#intro--details".into())
        );
        assert!(completion_new_texts(&project, page, &contexts[4]).contains(&"#source".into()));
        assert!(completion_new_texts(&project, page, &contexts[5]).contains(&"/logo.png".into()));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    fn completion_new_texts(
        project: &AuthoringProject,
        page: &AuthoringPage,
        context: &MarkdownTargetContext,
    ) -> Vec<String> {
        completion_items_for_markdown_target(project, page, context)
            .into_iter()
            .filter_map(|item| match item.text_edit {
                Some(CompletionTextEdit::Edit(edit)) => Some(edit.new_text),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn hovers_links_and_frontmatter_from_authoring_project() {
        let dir = temp_dir("hovers");
        let content_dir = dir.join("content");
        let static_dir = dir.join("static");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::create_dir_all(&static_dir).expect("create static dir");
        std::fs::write(static_dir.join("logo.png"), b"png").expect("write static");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n\n## Details\n",
        )
        .expect("write intro");
        let source = "\
+++
title = \"Source\"
+++

[intro](/guide/intro#intro--details)
[missing](/missing)
![logo](/logo.png)
";
        std::fs::write(content_dir.join("guide/source.md"), source).expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let page = project
            .page_for_source_file("guide/source.md")
            .expect("source page");

        let intro_reference = reference_at_position(source, position_for(source, "/guide/intro"))
            .expect("intro reference");
        let intro_hover = link_hover_markdown(&project, page, source, &intro_reference);
        assert!(intro_hover.contains("**Dodeca Intro**"));
        assert!(intro_hover.contains("> # Intro ## Details"));
        assert!(intro_hover.contains("**Heading**: H2 `Details` (`#intro--details`)"));
        assert!(intro_hover.contains("| `/guide/intro` | `guide/intro.md` |"));

        let missing_reference = reference_at_position(source, position_for(source, "/missing"))
            .expect("missing reference");
        let missing_hover = link_hover_markdown(&project, page, source, &missing_reference);
        assert!(missing_hover.contains("route '/missing' not found"));

        let logo_reference = reference_at_position(source, position_for(source, "/logo.png"))
            .expect("logo reference");
        let logo_hover = link_hover_markdown(&project, page, source, &logo_reference);
        assert!(logo_hover.contains("Dodeca static asset"));
        assert!(logo_hover.contains("static/logo.png"));

        let frontmatter_hover = frontmatter_hover_markdown(&project, page, source, 3);
        assert!(frontmatter_hover.contains("**Dodeca page: Source**"));
        assert!(frontmatter_hover.contains("> [intro](/guide/intro#intro--details)"));
        assert!(frontmatter_hover.contains("| route | source | headings | backlinks |"));
        assert!(frontmatter_hover.contains("| `/guide/source` | `guide/source.md` |"));
        assert!(frontmatter_hover.contains("| transforms | `markdown -> page template"));
        assert!(frontmatter_hover.contains("| static assets | `logo.png` |"));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn symbols_list_pages_and_headings_from_authoring_project() {
        let dir = temp_dir("symbols");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(content_dir.join("guide")).expect("create content dirs");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(
            content_dir.join("guide/intro.md"),
            "+++\ntitle = \"Intro\"\n+++\n\n# Intro\n\n## Details\n",
        )
        .expect("write intro");
        std::fs::write(content_dir.join("guide/source.md"), "# Source\n").expect("write source");

        let project = load_authoring_project(
            &content_dir,
            &[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Source("guide/draft.md".to_string()),
                content: "+++\ntitle = \"Draft\"\n+++\n\n# Draft\n".to_string(),
            }],
        )
        .await
        .expect("load project");
        let intro = project
            .page_for_source_file("guide/intro.md")
            .expect("intro page");
        let intro_content = project
            .source_contents
            .get("guide/intro.md")
            .expect("intro content");

        let document_symbols = document_symbol_for_page(intro, intro_content);
        let children = document_symbols.children.expect("heading children");
        assert_eq!(document_symbols.name, "Intro");
        assert_eq!(
            children
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Intro", "Details"]
        );

        let route_symbols = workspace_symbols_for_project(&content_dir, &project, "/guide/intro");
        assert!(route_symbols.iter().any(|symbol| symbol.name == "Intro"));

        let source_symbols = workspace_symbols_for_project(&content_dir, &project, "draft.md");
        assert!(source_symbols.iter().any(|symbol| symbol.name == "Draft"));

        let heading_symbols =
            workspace_symbols_for_project(&content_dir, &project, "intro--details");
        assert!(heading_symbols.iter().any(|symbol| {
            symbol.name == "Details"
                && symbol
                    .container_name
                    .as_deref()
                    .is_some_and(|container| container.contains("/guide/intro"))
        }));

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn authoring_project_uses_open_document_overlays() {
        let dir = temp_dir("overlays");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::write(content_dir.join("_index.md"), "[target](/target)\n").expect("write root");
        std::fs::write(content_dir.join("target.md"), "# Target\n").expect("write target");

        let disk_project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load disk project");
        assert!(disk_project.route_exists("/target"));

        let overlay_project = load_authoring_project(
            &content_dir,
            &[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Source("draft.md".to_string()),
                content: "+++\ntitle = \"Draft\"\n+++\n".to_string(),
            }],
        )
        .await
        .expect("load overlay project");

        assert!(overlay_project.route_exists("/target"));
        assert!(overlay_project.route_exists("/draft"));
        assert_eq!(
            overlay_project.source_file_for_route("/draft"),
            Some("draft.md")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn authoring_workspace_updates_open_document_overlays() {
        let dir = temp_dir("workspace-overlays");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
        assert!(
            !workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .route_exists("/draft")
        );

        workspace
            .apply_overlays(&[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Source("draft.md".to_string()),
                content: "+++\ntitle = \"Draft\"\n+++\n".to_string(),
            }])
            .expect("apply overlay");
        assert!(
            workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .route_exists("/draft")
        );

        workspace.apply_overlays(&[]).expect("clear overlay");
        assert!(
            !workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .route_exists("/draft")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn authoring_workspace_updates_template_overlays() {
        let dir = temp_dir("workspace-template-overlays");
        let content_dir = dir.join("content");
        let templates_dir = dir.join("templates");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::create_dir_all(&templates_dir).expect("create templates dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
        assert!(
            !workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .template_paths
                .contains_key("custom.html")
        );

        workspace
            .apply_overlays(&[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Template("custom.html".to_string()),
                content: "{% set alpha = \"/\" %}{{ alpha }}".to_string(),
            }])
            .expect("apply template overlay");
        let alpha_project = workspace.inputs().project().await.expect("alpha project");
        assert!(alpha_project.template_paths.contains_key("custom.html"));
        let alpha_index = alpha_project
            .template_semantics
            .get("custom.html")
            .expect("alpha template semantics");
        assert!(
            alpha_index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "alpha")
        );

        workspace
            .apply_overlays(&[AuthoringDocumentOverlay {
                path: AuthoringInputPath::Template("custom.html".to_string()),
                content: "{% set beta = \"/\" %}{{ beta".to_string(),
            }])
            .expect("update template overlay");
        let beta_project = workspace.inputs().project().await.expect("beta project");
        let beta_index = beta_project
            .template_semantics
            .get("custom.html")
            .expect("beta template semantics");
        assert!(
            beta_index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "beta")
        );
        assert!(
            !beta_index
                .symbols
                .iter()
                .any(|symbol| symbol.name == "alpha")
        );

        workspace.apply_overlays(&[]).expect("clear overlay");
        let cleared_project = workspace.inputs().project().await.expect("cleared project");
        assert!(!cleared_project.template_paths.contains_key("custom.html"));
        assert!(
            !cleared_project
                .template_semantics
                .contains_key("custom.html")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[tokio::test]
    async fn authoring_workspace_updates_disk_source_changes() {
        let dir = temp_dir("workspace-disk-source-changes");
        let content_dir = dir.join("content");
        std::fs::create_dir_all(&content_dir).expect("create content dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");

        let mut workspace = AuthoringWorkspace::new(&content_dir).expect("workspace");
        workspace
            .apply_file_change(
                &AuthoringInputPath::Source("draft.md".to_string()),
                Some("+++\ntitle = \"Draft\"\n+++\n"),
            )
            .expect("apply source change");
        assert!(
            workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .route_exists("/draft")
        );

        workspace
            .apply_file_change(&AuthoringInputPath::Source("draft.md".to_string()), None)
            .expect("apply source removal");
        assert!(
            !workspace
                .inputs()
                .project()
                .await
                .expect("project")
                .route_exists("/draft")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }
}
