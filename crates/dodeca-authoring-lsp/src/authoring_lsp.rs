use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::{Facet, NumericType, PrimitiveType, Type, UserType};
use gingembre::ast::{Expr, Ident, Node, StringLit};
use gingembre::semantic::{
    TemplateReferenceAccess, TemplateReferenceKind, TemplateSemanticIndex,
    TemplateSemanticTokenKind, TemplateSymbol, TemplateSymbolKind, TemplateSymbolOrigin,
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

// The route-graph analysis layer (link resolution, markdown refs, route graph)
// now lives in `dodeca::authoring_graph`; re-export so the rest of this module
// (and `ddc`) keep referencing the items unqualified.
pub use dodeca::authoring_graph::*;
// Template analysis types + span/range helpers now live in
// `dodeca::authoring_templates`; re-export so the rest of this module is unchanged.
pub use dodeca::authoring_model::{AuthoringDiagnostic, AuthoringDiagnosticKind};
use dodeca::authoring_model::{
    AuthoringDocumentOverlay, AuthoringInputPath, AuthoringPage, AuthoringPageKind,
    AuthoringProject, AuthoringWorkspace, RenderedHref, RenderedHrefOrigin,
};
pub use dodeca::authoring_templates::*;
use dodeca::config::{ResolvedConfig, ResolvedSource};
use dodeca::queries::{Frontmatter, default_title_from_source_path};

/// Resolve the source set for an authoring workspace rooted at `content_dir`.
///
/// Discovers the config from the workspace (the aggregator config, when you've
/// opened the aggregate) and uses its `sources` — so the LSP sees the full
/// multi-source world (cross-source links, files in mounted sub-repos). Also
/// publishes it as the global config so source/wiki resolution works. Falls
/// back to a single source for a config-less directory.
fn workspace_sources(content_dir: &camino::Utf8Path) -> Vec<ResolvedSource> {
    if let Ok(Some(cfg)) = ResolvedConfig::discover_containing(content_dir) {
        let _ = dodeca::config::set_global_config(cfg.clone());
        if !cfg.sources.is_empty() {
            return cfg.sources;
        }
    }
    vec![ResolvedSource {
        name: String::new(),
        mount: "/".to_string(),
        content_dir: content_dir.to_owned(),
        checkout_dir: None,
        git: None,
        repo: None,
        impls: Vec::new(),
    }]
}
use dodeca::template_host::TEMPLATE_FUNCTION_NAMES;
use dodeca::template_paths::{logical_template_path, physical_template_path};
use dodeca::types::SourcePath;

pub const LIST_PAGES_COMMAND: &str = "dodeca.listPages";
pub const DIAGNOSTICS_COMMAND: &str = "dodeca.authoringDiagnostics";
pub const CREATE_PAGE_COMMAND: &str = "dodeca.createPage";
pub const ROUTE_GRAPH_COMMAND: &str = "dodeca.routeGraph";

pub const TEMPLATE_SEMANTIC_TOKEN_VARIABLE: u32 = 0;
pub const TEMPLATE_SEMANTIC_TOKEN_PARAMETER: u32 = 1;
pub const TEMPLATE_SEMANTIC_TOKEN_PROPERTY: u32 = 2;
pub const TEMPLATE_SEMANTIC_TOKEN_FUNCTION: u32 = 3;
pub const TEMPLATE_SEMANTIC_TOKEN_MACRO: u32 = 4;
pub const TEMPLATE_SEMANTIC_TOKEN_STRING: u32 = 5;
pub const TEMPLATE_SEMANTIC_TOKEN_NUMBER: u32 = 6;
pub const TEMPLATE_SEMANTIC_TOKEN_KEYWORD: u32 = 7;

pub async fn run(content: Option<String>, output: Option<String>) -> Result<()> {
    serve_on(
        tokio::io::stdin(),
        tokio::io::stdout(),
        content,
        output,
        None,
    )
    .await;
    Ok(())
}

/// Like [`run`], but backed by a db-backed [`AuthoringProjectProvider`] (the
/// project is built from a loaded picante db with VFS overlays, not the disk
/// "world"). This is how the standalone `ddc lsp` gets the same incremental,
/// buffer-aware analysis as the in-process browser-editor LSP.
pub async fn run_with_provider(
    content: Option<String>,
    output: Option<String>,
    provider: Arc<dyn dodeca::authoring_model::AuthoringProjectProvider>,
) -> Result<()> {
    serve_on(
        tokio::io::stdin(),
        tokio::io::stdout(),
        content,
        output,
        Some(provider),
    )
    .await;
    Ok(())
}

/// Serve the authoring LSP over an arbitrary byte transport (LSP `Content-Length`
/// framing). `run` uses stdio; the in-process browser editor bridges this onto a
/// vox channel via an in-memory duplex, so no subprocess is spawned. When
/// `provider` is set, the project is built from the host's live db instead of a
/// disk workspace.
pub async fn serve_on<R, W>(
    read: R,
    write: W,
    content: Option<String>,
    output: Option<String>,
    provider: Option<Arc<dyn dodeca::authoring_model::AuthoringProjectProvider>>,
) where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let state = Arc::new(Mutex::new(AuthoringState {
        startup_args: LspStartupArgs { content, output },
        dirs: None,
        documents: HashMap::new(),
        input_revision: 0,
        workspace: None,
        applied_input_revision: None,
        world_cache: None,
        project_provider: provider,
    }));

    let (service, socket) = LspService::new(|client| Backend {
        client,
        state: Arc::clone(&state),
    });

    Server::new(read, write, socket).serve(service).await;
}

pub async fn authoring_diagnostics_for_content_dir(
    content_dir: &Utf8Path,
) -> Result<Vec<AuthoringDiagnostic>> {
    let workspace = AuthoringWorkspace::new(&workspace_sources(content_dir))?;
    let project = workspace.inputs().project().await?;
    let world = AuthoringWorld::new(project)?;
    Ok(load_authoring_diagnostics_for_world(&world))
}

#[derive(Clone)]
pub struct Backend {
    pub client: Client,
    pub state: Arc<Mutex<AuthoringState>>,
}

pub struct AuthoringState {
    pub startup_args: LspStartupArgs,
    pub dirs: Option<AuthoringDirs>,
    pub documents: HashMap<Url, String>,
    pub input_revision: u64,
    pub workspace: Option<AuthoringWorkspace>,
    pub applied_input_revision: Option<u64>,
    pub world_cache: Option<CachedAuthoringWorld>,
    /// When set (browser editor), build the project from the host's live db
    /// (snapshot + overlays) instead of loading a disk workspace.
    pub project_provider: Option<Arc<dyn dodeca::authoring_model::AuthoringProjectProvider>>,
}

#[derive(Debug, Clone)]
pub struct LspStartupArgs {
    pub content: Option<String>,
    pub output: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedAuthoringWorld {
    pub content_dir: Utf8PathBuf,
    pub input_revision: u64,
    pub world: AuthoringWorld,
}

#[derive(Debug, Clone)]
pub struct AuthoringWorld {
    pub project: AuthoringProject,
    pub template_index: TemplateAuthoringIndex,
    pub content_graph: ContentAuthoringGraph,
    pub source_document_targets: HashMap<String, Vec<FrontmatterDocumentTarget>>,
}

impl AuthoringWorld {
    pub fn new(project: AuthoringProject) -> Result<Self> {
        let content_graph = ContentAuthoringGraph::new(&project);
        Self::with_content_graph(project, content_graph)
    }

    /// Build the world with a precomputed content graph — the db-backed path
    /// supplies the memoized `content_graph` tracked-query result so the O(n)
    /// graph isn't recomputed by hand.
    pub fn with_content_graph(
        project: AuthoringProject,
        content_graph: ContentAuthoringGraph,
    ) -> Result<Self> {
        let template_index = TemplateAuthoringIndex::new(&project);
        let mut source_document_targets = HashMap::new();
        for (source_file, content) in &project.source_contents {
            source_document_targets.insert(
                source_file.clone(),
                frontmatter_document_targets(&project, content)?,
            );
        }
        Ok(Self {
            project,
            template_index,
            content_graph,
            source_document_targets,
        })
    }

    pub fn route_graph(&self) -> &[RouteGraphNode] {
        self.content_graph.routes()
    }

    pub fn inbound_reference_count(&self, route: &str) -> usize {
        self.content_graph.inbound_reference_count(route)
    }

    pub fn references_to_page(
        &self,
        content_dir: &Utf8Path,
        target_page: &AuthoringPage,
    ) -> Result<Vec<Location>> {
        references_to_page(content_dir, &self.project, target_page)
    }

    pub fn source_document_targets(&self, source_file: &str) -> &[FrontmatterDocumentTarget] {
        self.source_document_targets
            .get(source_file)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn source_document_target_at_position(
        &self,
        source_file: &str,
        position: Position,
    ) -> Option<FrontmatterDocumentTarget> {
        self.source_document_targets(source_file)
            .iter()
            .find(|target| range_contains_position(&target.source_range, position))
            .cloned()
    }

    pub fn template_document_references(
        &self,
        content_dir: &Utf8Path,
        target_path: &Utf8Path,
    ) -> Result<Vec<Location>> {
        let mut locations = Vec::new();
        for target in self.template_index.document_reference_targets(target_path) {
            locations.push(Location {
                uri: Url::from_file_path(target.path.as_std_path()).map_err(|_| {
                    eyre!(
                        "could not convert template document reference path to URI: {}",
                        target.path
                    )
                })?,
                range: target.range,
            });
        }

        for (source_file, targets) in &self.source_document_targets {
            for target in targets {
                if target.kind == FrontmatterDocumentKind::Template
                    && target.target_path == target_path
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
}

#[derive(Debug, Clone)]
pub struct ContentAuthoringGraph {
    pub routes: Vec<RouteGraphNode>,
}

impl ContentAuthoringGraph {
    pub fn new(project: &AuthoringProject) -> Self {
        Self {
            routes: route_graph_for_project(project),
        }
    }

    pub fn routes(&self) -> &[RouteGraphNode] {
        &self.routes
    }

    pub fn inbound_reference_count(&self, route: &str) -> usize {
        self.routes
            .iter()
            .find(|node| node.route == route)
            .map(|node| node.incoming.len())
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoringDirs {
    pub content_dir: Utf8PathBuf,
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
                version: Some(dodeca::dodeca_version().to_string()),
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
    pub fn set_dirs(&self, dirs: AuthoringDirs) {
        let mut state = self.state.lock().unwrap();
        if state.dirs.as_ref() != Some(&dirs) {
            state.workspace = None;
            state.applied_input_revision = None;
            state.world_cache = None;
        }
        state.dirs = Some(dirs);
    }

    pub fn resolve_dirs_from_initialize(
        &self,
        params: &InitializeParams,
    ) -> Result<Option<AuthoringDirs>> {
        let startup_args = self.state.lock().unwrap().startup_args.clone();
        resolve_initial_authoring_dirs(&startup_args, params)
    }

    pub fn dirs(&self) -> Result<AuthoringDirs> {
        let state = self.state.lock().unwrap();
        state
            .dirs
            .clone()
            .ok_or_else(|| eyre!("dodeca authoring server has not been initialized"))
    }

    pub fn dirs_for_uri(&self, uri: &Url) -> Result<AuthoringDirs> {
        let startup_args = self.state.lock().unwrap().startup_args.clone();
        let dirs = resolve_authoring_dirs_for_document(&startup_args, uri)?;
        self.set_dirs(dirs.clone());
        Ok(dirs)
    }

    #[allow(clippy::disallowed_types)]
    pub async fn register_workspace_file_watches(&self) {
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

    pub async fn apply_watched_file_changes(
        &self,
        params: DidChangeWatchedFilesParams,
    ) -> Result<()> {
        let dirs = self.dirs()?;
        let mut changed = false;
        {
            let mut state = self.state.lock().unwrap();
            let needs_workspace = state
                .workspace
                .as_ref()
                .is_none_or(|workspace| workspace.content_dir() != dirs.content_dir);

            if needs_workspace {
                state.workspace = Some(AuthoringWorkspace::new(&workspace_sources(
                    &dirs.content_dir,
                ))?);
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

    pub async fn list_pages(&self) -> Result<Vec<AuthoringPage>> {
        let dirs = self.dirs()?;
        Ok(self.current_project(&dirs).await?.pages)
    }

    pub async fn authoring_diagnostics(&self) -> Result<Vec<AuthoringDiagnostic>> {
        let dirs = self.dirs()?;
        let world = self.current_world(&dirs).await?;
        Ok(load_authoring_diagnostics_for_world(&world))
    }

    pub async fn authoring_route_graph(&self) -> Result<Vec<RouteGraphNode>> {
        let dirs = self.dirs()?;
        let world = self.current_world(&dirs).await?;
        Ok(world.route_graph().to_vec())
    }

    pub fn set_document(&self, uri: Url, content: String) {
        let mut state = self.state.lock().unwrap();
        state.documents.insert(uri, content);
        state.input_revision = state.input_revision.wrapping_add(1);
        state.world_cache = None;
    }

    pub fn remove_document(&self, uri: &Url) {
        let mut state = self.state.lock().unwrap();
        state.documents.remove(uri);
        state.input_revision = state.input_revision.wrapping_add(1);
        state.world_cache = None;
    }

    pub fn document_content(&self, uri: &Url) -> Result<String> {
        if let Some(content) = self.state.lock().unwrap().documents.get(uri).cloned() {
            return Ok(content);
        }

        let path = uri
            .to_file_path()
            .map_err(|_| eyre!("LSP document URI is not a file URI: {uri}"))?;
        Ok(std::fs::read_to_string(path)?)
    }

    pub async fn current_project(&self, dirs: &AuthoringDirs) -> Result<AuthoringProject> {
        Ok(self.current_world(dirs).await?.project)
    }

    pub async fn current_world(&self, dirs: &AuthoringDirs) -> Result<AuthoringWorld> {
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

        // Shared path: build the project from the host's live db (snapshot +
        // open-document overlays) instead of a disk workspace.
        let provider = self.state.lock().unwrap().project_provider.clone();
        if let Some(provider) = provider {
            let (overlays, revision) = {
                let state = self.state.lock().unwrap();
                let overlays = state
                    .documents
                    .iter()
                    .filter_map(|(uri, content)| {
                        Some((
                            lsp_file_uri_to_utf8_path(uri).ok()?.to_string(),
                            content.clone(),
                        ))
                    })
                    .collect::<Vec<_>>();
                (overlays, state.input_revision)
            };
            let snapshot = provider.snapshot(overlays).await?;
            let project = snapshot.project().await?;
            let content_graph = ContentAuthoringGraph {
                routes: snapshot.content_graph().await?,
            };
            let world = AuthoringWorld::with_content_graph(project, content_graph)?;
            let mut state = self.state.lock().unwrap();
            if state.input_revision == revision {
                state.world_cache = Some(CachedAuthoringWorld {
                    content_dir: dirs.content_dir.clone(),
                    input_revision: revision,
                    world: world.clone(),
                });
            }
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
                state.workspace = Some(AuthoringWorkspace::new(&workspace_sources(
                    &dirs.content_dir,
                ))?);
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
        let world = AuthoringWorld::new(project)?;

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

    pub async fn code_actions(&self, params: CodeActionParams) -> Result<CodeActionResponse> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let world = self.current_world(&dirs).await?;
        let project = &world.project;
        let path = lsp_file_uri_to_utf8_path(&uri)?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let diagnostics = world.template_index.diagnostics(&template_file).to_vec();
            let lsp_diagnostics = diagnostics
                .iter()
                .map(authoring_diagnostic_to_lsp)
                .collect::<Vec<_>>();
            let mut actions = Vec::new();
            for diagnostic in diagnostics
                .into_iter()
                .filter(|diagnostic| ranges_overlap(&diagnostic.range(), &params.range))
            {
                match diagnostic.kind {
                    AuthoringDiagnosticKind::MissingTemplate => {
                        actions.extend(missing_template_code_actions(
                            &dirs.content_dir,
                            &diagnostic,
                            &lsp_diagnostics,
                        )?)
                    }
                    AuthoringDiagnosticKind::Route
                    | AuthoringDiagnosticKind::Anchor
                    | AuthoringDiagnosticKind::Source
                    | AuthoringDiagnosticKind::StaticAsset
                    | AuthoringDiagnosticKind::Frontmatter
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
            return Ok(actions);
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;
        let diagnostics = diagnostics_for_page(project, page, &content);
        let lsp_diagnostics = diagnostics
            .iter()
            .map(authoring_diagnostic_to_lsp)
            .collect::<Vec<_>>();

        let mut actions = Vec::new();
        if let Some(action) =
            extract_page_code_action(&dirs.content_dir, project, &uri, &content, params.range)?
        {
            actions.push(action);
        }
        if let Some(action) = create_frontmatter_code_action(&uri, page, &content, params.range) {
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
                    project,
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

    pub async fn completions(&self, params: CompletionParams) -> Result<Vec<CompletionItem>> {
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

    pub async fn hover_for_position(&self, uri: &Url, position: Position) -> Result<Option<Hover>> {
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
                template_index.route_reference_at_position(&template_file, position)
                && let Some(target_page) = project.page_for_route(&reference.target_route)
            {
                let (_, fragment) = split_fragment(&reference.target);
                return Ok(Some(markdown_hover(
                    page_link_hover_markdown(project, target_page, fragment),
                    reference.source_range,
                )));
            }
            if let Some(target) =
                template_index.document_target_at_position(&template_file, position)
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
                template_semantic_hover(&world.template_index, &template_file, position)
            {
                return Ok(Some(hover));
            }
            return Ok(None);
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(target) = world.source_document_target_at_position(&source_file, position) {
            return Ok(Some(markdown_hover(
                target.hover_markdown(),
                target.source_range,
            )));
        }

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            let backlink_count = world.inbound_reference_count(&page.route);
            return Ok(Some(markdown_hover(
                frontmatter_hover_markdown(
                    project,
                    &world.template_index,
                    page,
                    &content,
                    backlink_count,
                ),
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

    pub async fn document_links(&self, params: DocumentLinkParams) -> Result<Vec<DocumentLink>> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let world = self.current_world(&dirs).await?;
        let project = &world.project;
        let path = lsp_file_uri_to_utf8_path(&uri)?;

        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let mut links = world
                .template_index
                .document_targets(&template_file)
                .iter()
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
                world
                    .template_index
                    .route_references(&template_file)
                    .iter()
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

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        world
            .source_document_targets(&source_file)
            .iter()
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

    pub async fn semantic_tokens_for_document(&self, uri: &Url) -> Result<Option<SemanticTokens>> {
        let dirs = self.dirs_for_uri(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? else {
            return Ok(None);
        };
        Ok(self
            .current_world(&dirs)
            .await?
            .template_index
            .semantic_tokens(&template_file))
    }

    pub async fn document_symbols(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Vec<DocumentSymbol>> {
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

    pub async fn workspace_symbols(
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

    pub async fn definition_for_position(
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
                template_index.route_reference_at_position(&template_file, position)
            {
                let (_, fragment) = split_fragment(&reference.target);
                if let Some(source_file) = project.source_file_for_route(&reference.target_route) {
                    let path = dirs.content_dir.join(source_file);
                    return location_for_source_path(&path, fragment);
                }
            }
            if let Some(target) =
                template_index.document_target_at_position(&template_file, position)
            {
                return Ok(Some(Location {
                    uri: target.target_uri()?,
                    range: one_line_range(0),
                }));
            }
            if let Some(query) = template_index.macro_reference_query(&template_file, position)
                && let Some(target) = template_index
                    .macro_definition_target(&query.target_template_file, &query.macro_name)
            {
                return Ok(Some(Location {
                    uri: Url::from_file_path(target.path.as_std_path()).map_err(|_| {
                        eyre!(
                            "could not convert template macro definition path to URI: {}",
                            target.path
                        )
                    })?,
                    range: target.range,
                }));
            }
            if let Some(target) =
                template_definition_target_at_position(project, &template_file, &content, position)?
            {
                return Ok(Some(target.location()?));
            }
            if let Some(location) = template_index.semantic_definition(&template_file, position) {
                return Ok(Some(location));
            }
            return Ok(None);
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;

        if let Some(target) = world.source_document_target_at_position(&source_file, position) {
            return Ok(Some(Location {
                uri: target.target_uri()?,
                range: one_line_range(0),
            }));
        }

        let Some(reference) = reference_at_position(&content, position) else {
            return Ok(None);
        };

        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        definition_for_reference(&dirs, project, page, &reference)
    }

    pub async fn references_for_position(
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
                template_index.document_target_at_position(&template_file, position)
            {
                return world.template_document_references(&dirs.content_dir, &target.target_path);
            }
            if let Some(query) = template_index.macro_reference_query(&template_file, position) {
                return template_macro_references(
                    template_index,
                    &query.target_template_file,
                    &query.macro_name,
                );
            }
            if let Some(reference) =
                template_index.route_reference_at_position(&template_file, position)
                && let Some(target_page) = project.page_for_route(&reference.target_route)
            {
                return world.references_to_page(&dirs.content_dir, target_page);
            }
            return Ok(template_index.semantic_references(&template_file, position));
        }

        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(target) = world.source_document_target_at_position(&source_file, position)
            && target.kind == FrontmatterDocumentKind::Template
        {
            return world.template_document_references(&dirs.content_dir, &target.target_path);
        }

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            return world.references_to_page(&dirs.content_dir, page);
        }

        if let Some(heading_id) = heading_id_at_position(page, &content, position) {
            return references_to_heading(&dirs.content_dir, project, page, &heading_id);
        }

        Ok(Vec::new())
    }

    pub async fn prepare_rename_for_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Option<PrepareRenameResponse>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let world = self.current_world(&dirs).await?;
            if let Some(occurrence) = world
                .template_index
                .block_occurrence_at_position(&template_file, position)
            {
                return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: occurrence.source_range,
                    placeholder: occurrence.name,
                }));
            }
            if let Some(query) = world
                .template_index
                .macro_reference_query(&template_file, position)
            {
                return Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                    range: query.source_range,
                    placeholder: query.macro_name,
                }));
            }
            return Ok(template_semantic_prepare_rename(
                &world.template_index,
                &template_file,
                position,
            ));
        }

        let project = self.current_project(&dirs).await?;

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

    pub async fn rename_for_position(
        &self,
        uri: &Url,
        position: Position,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        if let Some(template_file) = template_file_for_path(&dirs.content_dir, &path)? {
            let world = self.current_world(&dirs).await?;
            if let Some(occurrence) = world
                .template_index
                .block_occurrence_at_position(&template_file, position)
            {
                return template_block_rename_workspace_edit(
                    &world.template_index,
                    &template_file,
                    &occurrence.name,
                    new_name,
                );
            }
            if let Some(query) = world
                .template_index
                .macro_reference_query(&template_file, position)
            {
                return template_macro_rename_workspace_edit(
                    &world.template_index,
                    &query.target_template_file,
                    &query.macro_name,
                    new_name,
                );
            }
            return template_semantic_rename_workspace_edit(
                &world.template_index,
                &template_file,
                position,
                new_name,
            );
        }

        let project = self.current_project(&dirs).await?;

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

    pub async fn publish_document_diagnostics(&self, uri: Url, content: String) {
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
        let world = match self.current_world(&dirs).await {
            Ok(world) => world,
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
        let project = &world.project;
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
            let diagnostics = world
                .template_index
                .diagnostics(&template_file)
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
        let diagnostics = match diagnostics_for_uri(&dirs.content_dir, project, &uri, &content) {
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

    pub async fn publish_workspace_diagnostics(&self) {
        let dirs = match self.dirs() {
            Ok(dirs) => dirs,
            Err(_) => return,
        };
        let world = match self.current_world(&dirs).await {
            Ok(world) => world,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                return;
            }
        };
        let project = &world.project;
        let diagnostics = load_authoring_diagnostics_for_world(&world);
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
    pub async fn create_page_from_command(
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

pub fn resolve_initial_authoring_dirs(
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

pub fn resolve_authoring_dirs_for_document(
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

pub fn authoring_dirs_from_resolved_config(
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

pub fn authoring_dirs_from_config(content_dir: Utf8PathBuf) -> AuthoringDirs {
    AuthoringDirs { content_dir }
}

#[allow(deprecated)]
pub fn project_path_from_initialize(params: &InitializeParams) -> Result<Option<String>> {
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

pub fn lsp_file_uri_to_utf8_path(uri: &Url) -> Result<Utf8PathBuf> {
    let path = uri
        .to_file_path()
        .map_err(|_| eyre!("LSP workspace URI is not a file URI: {uri}"))?;
    Utf8PathBuf::from_path_buf(path)
        .map_err(|path| eyre!("LSP workspace path is not UTF-8: {}", path.display()))
}

#[derive(Debug, Clone)]
pub struct TemplateAuthoringIndex {
    pub templates: HashMap<String, IndexedTemplate>,
    pub children_by_parent: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct IndexedTemplate {
    pub path: Utf8PathBuf,
    pub content: String,
    pub semantic: Option<TemplateSemanticIndex>,
    pub extends: Option<String>,
    pub dependencies: Vec<String>,
    pub diagnostics: Vec<AuthoringDiagnostic>,
    pub document_targets: Vec<TemplateDocumentTarget>,
    pub route_references: Vec<TemplateRouteReference>,
    pub blocks: Vec<TemplateBlockOccurrence>,
    pub macros: Vec<TemplateMacroOccurrence>,
    pub macro_calls: Vec<TemplateMacroCallOccurrence>,
}

/// tower_lsp `Range` for an [`AuthoringDiagnostic`] (the type now lives in
/// `dodeca::authoring_model`; this extension keeps `diagnostic.range()` working
/// at call sites). Auto-in-scope for this module.
pub trait AuthoringDiagnosticExt {
    fn range(&self) -> Range;
}

impl AuthoringDiagnosticExt for AuthoringDiagnostic {
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

pub fn load_authoring_diagnostics_for_world(world: &AuthoringWorld) -> Vec<AuthoringDiagnostic> {
    let project = &world.project;
    let mut diagnostics = Vec::new();

    for page in &project.pages {
        let Some(content) = project.source_contents.get(&page.source_file) else {
            continue;
        };
        diagnostics.extend(diagnostics_for_page(project, page, content));
    }

    diagnostics.extend(world.template_index.all_diagnostics());
    diagnostics.extend(site_graph_diagnostics(project));

    diagnostics.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then_with(|| a.byte_start.cmp(&b.byte_start))
            .then_with(|| a.target.cmp(&b.target))
    });
    diagnostics
}

pub fn diagnostics_for_uri(
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

pub fn diagnostics_for_page(
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

pub fn diagnostics_for_template(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Vec<AuthoringDiagnostic> {
    let Ok(template) = gingembre::parse_template(template_file, content) else {
        return Vec::new();
    };

    diagnostics_for_template_nodes(project, template_file, content, &template.body)
}

pub fn diagnostics_for_template_nodes(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    nodes: &[Node],
) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    let imports = template_import_aliases(project, nodes);
    let parent_file = template_extends_path_from_nodes(nodes)
        .filter(|path| project.template_paths.contains_key(path));
    collect_template_diagnostics(
        project,
        template_file,
        content,
        nodes,
        parent_file.as_deref(),
        &imports,
        &mut diagnostics,
    );
    diagnostics
}

pub fn site_graph_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
    let mut diagnostics = Vec::new();
    diagnostics.extend(duplicate_title_diagnostics(project));
    diagnostics.extend(duplicate_route_diagnostics(project));
    diagnostics.extend(inbound_link_diagnostics(project));
    diagnostics
}

pub fn duplicate_title_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
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

pub fn duplicate_route_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
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

pub fn inbound_link_diagnostics(project: &AuthoringProject) -> Vec<AuthoringDiagnostic> {
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

pub fn inbound_link_counts(project: &AuthoringProject) -> HashMap<String, usize> {
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

pub fn is_section_landing_with_children(project: &AuthoringProject, page: &AuthoringPage) -> bool {
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

pub fn site_graph_diagnostic_for_page(
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

pub fn site_graph_page_identity_byte_range(content: &str) -> (usize, usize) {
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

pub fn collect_template_diagnostics(
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

pub fn push_missing_template_diagnostic(
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

pub fn collect_template_expr_diagnostics(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    expr: &Expr,
    imports: &HashMap<String, String>,
    diagnostics: &mut Vec<AuthoringDiagnostic>,
) {
    match expr {
        Expr::Optional(inner) => collect_template_expr_diagnostics(
            project,
            template_file,
            content,
            &inner.expr,
            imports,
            diagnostics,
        ),
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

pub fn collect_template_literal_diagnostics(
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

pub fn template_diagnostic_for_ident(
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

pub fn template_diagnostic_for_span(
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

pub fn best_effort_frontmatter_diagnostics_for_uri(
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

pub fn frontmatter_diagnostics_for_source(
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

pub fn frontmatter_diagnostic(
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

pub fn diagnostic_for_reference(
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
                return Some(markdown_reference_diagnostic(
                    &reference,
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

    Some(markdown_reference_diagnostic(
        &reference,
        page,
        content,
        kind,
        resolved_route,
        message,
    ))
}

pub fn missing_route_code_actions(
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

pub fn missing_template_code_actions(
    content_dir: &Utf8Path,
    diagnostic: &AuthoringDiagnostic,
    lsp_diagnostics: &[Diagnostic],
) -> Result<Vec<CodeActionOrCommand>> {
    let template_file = diagnostic.target.as_str();
    if !is_creatable_template_path(template_file) {
        return Ok(Vec::new());
    }
    let uri = template_file_uri(content_dir, template_file)?;
    let title = format!("Create template '{}'", template_file);
    Ok(vec![CodeActionOrCommand::CodeAction(CodeAction {
        title,
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(lsp_diagnostics_for_range(
            lsp_diagnostics,
            diagnostic.range(),
        )),
        edit: Some(WorkspaceEdit {
            changes: None,
            document_changes: Some(DocumentChanges::Operations(vec![
                DocumentChangeOperation::Op(ResourceOp::Create(CreateFile {
                    uri,
                    options: Some(CreateFileOptions {
                        overwrite: Some(false),
                        ignore_if_exists: Some(false),
                    }),
                    annotation_id: None,
                })),
            ])),
            change_annotations: None,
        }),
        command: None,
        is_preferred: Some(true),
        ..CodeAction::default()
    })])
}

pub fn is_creatable_template_path(path: &str) -> bool {
    !path.is_empty()
        && path.ends_with(".html")
        && !path.starts_with('/')
        && !path.split('/').any(|part| part.is_empty() || part == "..")
}

pub fn create_frontmatter_code_action(
    uri: &Url,
    page: &AuthoringPage,
    content: &str,
    range: Range,
) -> Option<CodeActionOrCommand> {
    if frontmatter_lsp_range(content).is_some() || range.start.line > 1 {
        return None;
    }
    let mut new_text = page_frontmatter(&page.title);
    if !content.is_empty() {
        new_text.push('\n');
    }
    Some(CodeActionOrCommand::CodeAction(CodeAction {
        title: "Create frontmatter".to_string(),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: None,
        edit: Some(WorkspaceEdit::new(HashMap::from([(
            uri.clone(),
            vec![TextEdit::new(
                Range {
                    start: Position::new(0, 0),
                    end: Position::new(0, 0),
                },
                new_text,
            )],
        )]))),
        command: None,
        is_preferred: Some(false),
        ..CodeAction::default()
    }))
}

pub fn missing_anchor_code_actions(
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

pub fn existing_anchor_code_actions(
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

pub fn create_anchor_code_action(
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

pub fn lsp_diagnostics_for_range(lsp_diagnostics: &[Diagnostic], range: Range) -> Vec<Diagnostic> {
    lsp_diagnostics
        .iter()
        .filter(|lsp_diagnostic| lsp_diagnostic.range == range)
        .cloned()
        .collect()
}

pub fn extract_page_code_action(
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

pub fn link_hover_markdown(
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

pub fn page_link_hover_markdown(
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

pub fn frontmatter_hover_markdown(
    project: &AuthoringProject,
    template_index: &TemplateAuthoringIndex,
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
    sections.push(build_provenance_hover_table(
        project,
        template_index,
        page,
        content,
    ));
    sections.join("\n\n")
}

pub fn page_hover_metadata_table(page: &AuthoringPage) -> String {
    format!(
        "| route | source | template | output |\n| --- | --- | --- | --- |\n| `{}` | `{}` | `{}` | `{}` |",
        page.route, page.source_file, page.template, page.output_path
    )
}

pub fn frontmatter_hover_metadata_table(page: &AuthoringPage, backlink_count: usize) -> String {
    format!(
        "| route | source | headings | backlinks |\n| --- | --- | ---: | ---: |\n| `{}` | `{}` | `{}` | `{}` |",
        page.route,
        page.source_file,
        page.heading_ids.len(),
        backlink_count
    )
}

pub fn build_provenance_hover_table(
    project: &AuthoringProject,
    template_index: &TemplateAuthoringIndex,
    page: &AuthoringPage,
    content: &str,
) -> String {
    let mut rows = vec![
        "| build | value |".to_string(),
        "| --- | --- |".to_string(),
        format!("| transforms | `{}` |", page_transform_chain(page)),
    ];

    let templates = template_index.dependency_names(&page.template);
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

pub fn markdown_content_excerpt(content: &str) -> Option<String> {
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

pub fn markdown_body_without_frontmatter(content: &str) -> &str {
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

pub fn collapse_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn page_transform_chain(page: &AuthoringPage) -> &'static str {
    match page.kind {
        AuthoringPageKind::Page => "markdown -> page template -> html postprocess -> output",
        AuthoringPageKind::Section => "markdown -> section template -> html postprocess -> output",
    }
}

pub fn static_asset_references(
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

pub fn markdown_hover(markdown: String, range: Range) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: Some(range),
    }
}

pub fn template_semantic_hover(
    template_index: &TemplateAuthoringIndex,
    template_file: &str,
    position: Position,
) -> Option<Hover> {
    let template = template_index.templates.get(template_file)?;
    let content = &template.content;
    let offset = position_to_byte_offset(content, position)?;
    let index = template.semantic.as_ref()?;
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
                template_symbol_reference_hover_markdown(index, symbol, reference.access)
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

pub fn template_semantic_prepare_rename(
    template_index: &TemplateAuthoringIndex,
    template_file: &str,
    position: Position,
) -> Option<PrepareRenameResponse> {
    let template = template_index.templates.get(template_file)?;
    let content = &template.content;
    let offset = position_to_byte_offset(content, position)?;
    let index = template.semantic.as_ref()?;
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

pub fn template_semantic_rename_workspace_edit(
    template_index: &TemplateAuthoringIndex,
    template_file: &str,
    position: Position,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    if !is_valid_template_rename_name(new_name) {
        return Ok(None);
    }
    let Some(template) = template_index.templates.get(template_file) else {
        return Ok(None);
    };
    let content = &template.content;
    let Some(offset) = position_to_byte_offset(content, position) else {
        return Ok(None);
    };
    let Some(index) = template.semantic.as_ref() else {
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
    let uri = Url::from_file_path(template.path.as_std_path())
        .map_err(|_| eyre!("could not convert template path to URI: {}", template.path))?;
    changes.insert(uri, edits);
    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

pub fn template_symbol_can_rename(symbol: &TemplateSymbol) -> bool {
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

pub fn is_valid_template_rename_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub fn template_symbol_hover_markdown(symbol: &TemplateSymbol) -> String {
    let info = template_symbol_info(symbol);
    format!(
        "**{}** `{}`\n\n{}",
        info.detail, symbol.name, info.documentation
    )
}

pub fn template_symbol_reference_hover_markdown(
    index: &TemplateSemanticIndex,
    symbol: &TemplateSymbol,
    access: TemplateReferenceAccess,
) -> String {
    let info = template_symbol_info(symbol);
    let read_count = index.read_references_to_symbol(symbol.id).len();
    let write_count = index.write_references_to_symbol(symbol.id).len();
    let access_label = match access {
        TemplateReferenceAccess::Read => "Read",
        TemplateReferenceAccess::Write => "Write",
    };
    format!(
        "**{}** `{}`\n\n{}\n\n{} reference. {read_count} read reference(s), {write_count} write reference(s).",
        info.detail, symbol.name, info.documentation, access_label
    )
}

pub fn template_field_hover_markdown(
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

pub fn template_item_hover_markdown(name: &str, info: TemplateItemInfo) -> String {
    format!("**{}** `{}`\n\n{}", info.detail, name, info.documentation)
}

pub fn template_semantic_tokens(
    index: &TemplateSemanticIndex,
    content: &str,
) -> Vec<SemanticToken> {
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

pub fn template_semantic_tokens_legend() -> SemanticTokensLegend {
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

pub fn template_semantic_token_type(kind: TemplateSemanticTokenKind) -> u32 {
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
pub fn document_symbol_for_page(page: &AuthoringPage, content: &str) -> DocumentSymbol {
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

pub fn workspace_symbols_for_project(
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

pub fn workspace_page_matches(page: &AuthoringPage, query: &str) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let haystack = format!(
        "{} {} {} {} {}",
        page.title, page.route, page.source_file, page.template, page.output_path
    );
    fuzzy_contains(&haystack, query)
}

pub fn workspace_heading_matches(
    page: &AuthoringPage,
    heading: &dodeca::authoring_model::AuthoringHeading,
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

pub fn fuzzy_contains(haystack: &str, query: &str) -> bool {
    let haystack = haystack.to_lowercase();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .all(|part| haystack.contains(&part))
}

#[allow(deprecated)]
pub fn symbol_information_for_page(
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
pub fn symbol_information_for_heading(
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

pub fn location_for_page(
    content_dir: &Utf8Path,
    project: &AuthoringProject,
    page: &AuthoringPage,
) -> Option<Location> {
    let uri = Url::from_file_path(content_dir.join(&page.source_file)).ok()?;
    let content = project.source_contents.get(&page.source_file)?;
    let range = frontmatter_lsp_range(content).unwrap_or_else(|| one_line_range(0));
    Some(Location { uri, range })
}

pub fn location_for_page_heading(
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

pub fn full_document_range(content: &str) -> Range {
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

pub fn one_line_range(line: u32) -> Range {
    Range {
        start: Position { line, character: 0 },
        end: Position { line, character: 0 },
    }
}

pub fn definition_for_reference(
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

pub fn references_to_page(
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

pub fn location_for_rendered_href_origin(
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
            project.template_paths.get(template_file)?.clone(),
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

pub fn references_to_heading(
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

pub fn rename_heading_workspace_edit(
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

pub fn heading_id_after_rename(
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

pub fn fragment_range_for_target_context(
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

pub fn missing_anchor_fragment(target: &str) -> Option<&str> {
    split_fragment(target)
        .1
        .filter(|fragment| !fragment.is_empty())
}

pub fn target_context_for_diagnostic(
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

pub fn anchor_suggestion_score(fragment: &str, heading_id: &str, heading_title: &str) -> usize {
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

pub fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingAnchorHeadingEdit {
    pub range: Range,
    pub new_text: String,
}

pub fn create_missing_anchor_heading_edit(
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

pub fn missing_anchor_heading_insertion(
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

pub fn missing_anchor_parent_and_leaf(fragment: &str) -> (Option<&str>, &str) {
    match fragment.rfind("--") {
        Some(idx) => (Some(&fragment[..idx]), &fragment[idx + 2..]),
        None => (None, fragment),
    }
}

pub fn title_from_slug(slug: &str) -> Option<String> {
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

pub fn insertion_position_after_line(content: &str, one_based_line: u32) -> Position {
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
pub struct ExtractPagePlan {
    pub source_file: String,
    pub new_source_file: String,
    pub new_route: String,
    pub title: String,
    pub new_content: String,
    pub replacement: String,
    pub selection: Range,
}

pub fn extract_page_plan(
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
pub struct ExtractedPageContent {
    pub title: String,
    pub slug: String,
    pub body: String,
}

pub fn extracted_page_content(selected: &str) -> ExtractedPageContent {
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

pub fn extract_leading_heading(selected: &str) -> Option<(String, &str)> {
    let leading_len = selected.len() - selected.trim_start_matches([' ', '\t', '\n', '\r']).len();
    let after_leading = &selected[leading_len..];
    let line_end = after_leading.find('\n').unwrap_or(after_leading.len());
    let first_line = &after_leading[..line_end];
    let title = title_from_atx_heading_line(first_line)?;
    let body_start = leading_len + line_end + usize::from(line_end < after_leading.len());
    Some((title, &selected[body_start..]))
}

pub fn title_from_atx_heading_line(line: &str) -> Option<String> {
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

pub fn title_from_selection(selected: &str) -> String {
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

pub fn normalize_extracted_body(body: &str) -> String {
    let body = body.trim_matches(|ch| matches!(ch, '\n' | '\r'));
    if body.trim().is_empty() {
        "\n".to_string()
    } else {
        format!("\n{}\n", body.trim())
    }
}

pub fn unique_extracted_source_file(
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

pub fn workspace_edit_for_extract_page(
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
pub struct PageRouteRenamePlan {
    pub old_route: String,
    pub new_route: String,
    pub old_source_file: String,
    pub new_source_file: String,
    pub text_edits: Vec<PageRouteTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRouteTextEdit {
    pub path: AuthoringInputPath,
    pub range: Range,
    pub new_target: String,
}

pub fn rename_page_route_workspace_edit(
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

pub fn page_route_rename_plan(
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
pub struct PageRouteRenameTarget {
    pub route: String,
    pub source_file: String,
}

pub fn page_route_rename_target(
    page: &AuthoringPage,
    new_name: &str,
) -> Option<PageRouteRenameTarget> {
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

pub fn page_route_rendered_href_edit(
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

pub fn rendered_href_origin_range(
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

pub fn page_route_link_edit(
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

pub fn page_route_text_edit_sort_key(path: &AuthoringInputPath) -> String {
    match path {
        AuthoringInputPath::Source(path) => format!("source:{path}"),
        AuthoringInputPath::Template(path) => format!("template:{path}"),
        AuthoringInputPath::Sass(path) => format!("sass:{path}"),
        AuthoringInputPath::Static(path) => format!("static:{path}"),
        AuthoringInputPath::Dist(path) => format!("dist:{path}"),
        AuthoringInputPath::Data(path) => format!("data:{path}"),
    }
}

pub fn workspace_edit_for_page_route_rename(
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

pub fn page_route_edit_uri(
    content_dir: &Utf8Path,
    path: &AuthoringInputPath,
    plan: &PageRouteRenamePlan,
) -> Result<Url> {
    match path {
        AuthoringInputPath::Source(source_file) if source_file == &plan.new_source_file => {
            Ok(source_file_uri(content_dir, &plan.new_source_file)?)
        }
        AuthoringInputPath::Source(source_file) => source_file_uri(content_dir, source_file),
        AuthoringInputPath::Template(template_file) => {
            template_file_uri(content_dir, template_file)
        }
        AuthoringInputPath::Sass(path)
        | AuthoringInputPath::Static(path)
        | AuthoringInputPath::Dist(path)
        | AuthoringInputPath::Data(path) => Err(eyre!(
            "page route rename cannot edit unsupported authoring input: {path}"
        )),
    }
}

pub fn source_file_uri(content_dir: &Utf8Path, source_file: &str) -> Result<Url> {
    Url::from_file_path(content_dir.join(source_file))
        .map_err(|_| eyre!("could not convert source file to URI: {source_file}"))
}

pub fn template_file_uri(content_dir: &Utf8Path, template_file: &str) -> Result<Url> {
    let project_dir = content_dir.parent().unwrap_or(content_dir);
    Url::from_file_path(physical_template_path(
        &project_dir.join("templates"),
        template_file,
    ))
    .map_err(|_| eyre!("could not convert template file to URI: {template_file}"))
}

pub fn target_base_and_suffix(target: &str) -> (&str, &str) {
    let suffix_start = [target.find('#'), target.find('?')]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(target.len());
    (&target[..suffix_start], &target[suffix_start..])
}

pub fn source_file_for_page_route(route: &str, kind: AuthoringPageKind) -> Option<String> {
    match kind {
        AuthoringPageKind::Page => source_file_for_new_route(route),
        AuthoringPageKind::Section => source_file_for_new_section_route(route),
    }
}

pub fn source_file_for_new_section_route(route: &str) -> Option<String> {
    let route = normalize_route(route);
    let relative = route.strip_prefix('/')?;
    if relative.is_empty() {
        return None;
    }
    validate_route_relative_path(relative)?;
    Some(format!("{relative}/_index.md"))
}

pub fn validate_markdown_source_file(source_file: &str) -> Option<()> {
    source_file.ends_with(".md").then_some(())?;
    if source_file.starts_with('/') {
        return None;
    }
    validate_route_relative_path(source_file.strip_suffix(".md").unwrap_or(source_file))
}

pub fn validate_route_relative_path(relative: &str) -> Option<()> {
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

pub fn link_base_route_for_route(route: &str, kind: AuthoringPageKind) -> String {
    match kind {
        AuthoringPageKind::Page => route_parent(route),
        AuthoringPageKind::Section => normalize_route(route),
    }
}

pub fn route_parent(route: &str) -> String {
    let route = normalize_route(route);
    let parts = route_segments(&route);
    if parts.len() <= 1 {
        "/".to_string()
    } else {
        format!("/{}", parts[..parts.len() - 1].join("/"))
    }
}

pub fn relative_source_path_from_source(source_file: &str, target_source_file: &str) -> String {
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

pub fn location_for_markdown_reference(
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

pub fn location_for_source_path(
    path: &Utf8Path,
    fragment: Option<&str>,
) -> Result<Option<Location>> {
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

pub fn location_for_path(path: &Utf8Path, line: u32, column: u32) -> Option<Location> {
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

pub fn location_for_static_target(
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

/// Build an [`AuthoringDiagnostic`] anchored at a markdown reference's span.
/// (Was `MarkdownReference::diagnostic`; the type now lives in
/// `dodeca::authoring_graph`, so this is a free fn over `AuthoringDiagnostic`,
/// which stays in this crate.)
pub fn markdown_reference_diagnostic(
    reference: &MarkdownReference,
    page: &AuthoringPage,
    content: &str,
    kind: AuthoringDiagnosticKind,
    resolved_route: Option<String>,
    message: String,
) -> AuthoringDiagnostic {
    let (line, column) = byte_to_line_column(content, reference.byte_start);
    let (line_end, column_end) = byte_to_line_column(content, reference.byte_end);
    AuthoringDiagnostic {
        source_file: page.source_file.clone(),
        route: page.route.clone(),
        kind,
        target: reference.target.clone(),
        resolved_route,
        message,
        line,
        column,
        line_end,
        column_end,
        byte_start: reference.byte_start,
        byte_end: reference.byte_end,
    }
}

pub fn reference_at_position(content: &str, position: Position) -> Option<MarkdownReference> {
    let byte_offset = position_to_byte_offset(content, position)?;
    markdown_references(content)
        .into_iter()
        .find(|reference| reference.byte_start <= byte_offset && byte_offset <= reference.byte_end)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownTargetContext {
    pub kind: MarkdownReferenceKind,
    pub target: String,
    pub range: Range,
    pub byte_start: usize,
    pub byte_end: usize,
}

pub fn markdown_target_context_at_position(
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

pub fn markdown_target_contexts(content: &str) -> Vec<MarkdownTargetContext> {
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

pub fn markdown_reference_kind_before_link_close(
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

pub fn completion_items_for_markdown_target(
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

pub fn route_completion_items(
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

pub fn source_completion_items(
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

pub fn static_completion_items(
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

pub fn heading_completion_items(
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

pub fn completion_item(
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

pub fn template_completion_item(
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
pub enum TemplateCompletionKind {
    Root,
    Field(Vec<String>),
    Filter,
    Test,
    Macro(String),
}

#[derive(Debug, Clone)]
pub struct TemplateCompletionContext {
    pub kind: TemplateCompletionKind,
    pub range: Range,
}

pub const TEMPLATE_ROOT_VALUES: &[&str] =
    &["config", "page", "section", "current_path", "root", "data"];
pub const TEMPLATE_CONFIG_FIELDS: &[&str] = &["title", "description", "base_url"];
pub const TEMPLATE_PAGE_FIELDS: &[&str] = &[
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
pub const TEMPLATE_SECTION_FIELDS: &[&str] = &[
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
pub struct TemplateItemInfo {
    pub detail: &'static str,
    pub documentation: &'static str,
}

pub fn template_root_info(name: &str) -> TemplateItemInfo {
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

pub fn template_field_info(path: &[String], name: &str) -> TemplateItemInfo {
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

pub fn template_function_info(name: &str) -> TemplateItemInfo {
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

pub fn template_filter_info(name: &str) -> TemplateItemInfo {
    builtin_filter(name)
        .map(template_builtin_info)
        .unwrap_or(TemplateItemInfo {
            detail: "Gingembre filter",
            documentation: "Built-in Gingembre filter.",
        })
}

pub fn template_test_info(name: &str) -> TemplateItemInfo {
    builtin_test(name)
        .map(template_builtin_info)
        .unwrap_or(TemplateItemInfo {
            detail: "Gingembre test",
            documentation: "Built-in Gingembre test.",
        })
}

pub fn template_builtin_info(info: &BuiltinItemInfo) -> TemplateItemInfo {
    TemplateItemInfo {
        detail: info.detail,
        documentation: info.documentation,
    }
}

pub fn template_symbol_info(symbol: &TemplateSymbol) -> TemplateItemInfo {
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

pub fn completion_kind_for_template_symbol(kind: TemplateSymbolKind) -> CompletionItemKind {
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

pub fn template_completion_items(
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

pub fn template_root_completion_items(
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

    let import_aliases = gingembre::parse_template(template_file, content)
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

    if let Ok(template) = gingembre::parse_template(template_file, content)
        && top_level_macro_names(&template.body).next().is_some()
    {
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

    items
}

pub fn template_field_completion_items(
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

pub fn template_resolved_field_path(
    project: &AuthoringProject,
    template_file: &str,
    path: &[String],
    offset: Option<usize>,
) -> Option<Vec<String>> {
    let offset = offset?;
    let index = project.template_semantics.get(template_file)?;
    resolve_template_expression_path(index, path, offset, 0)
}

pub fn resolve_template_expression_path(
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

pub fn resolve_template_symbol_path(
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

pub fn template_iteration_item_path(iter_path: &[String]) -> Option<Vec<String>> {
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

pub fn template_macro_completion_items(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    namespace: &str,
    range: Range,
) -> Vec<CompletionItem> {
    let target_file = if namespace == "self" {
        Some(template_file.to_string())
    } else {
        gingembre::parse_template(template_file, content)
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
    let Ok(template) = gingembre::parse_template(target_file, target_content) else {
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

pub fn top_level_macro_names(nodes: &[Node]) -> impl Iterator<Item = String> + '_ {
    nodes.iter().filter_map(|node| match node {
        Node::Macro(node) => Some(node.name.name.clone()),
        _ => None,
    })
}

pub fn template_completion_context(
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

pub fn template_macro_completion_namespace(tag_prefix: &str) -> Option<String> {
    let before_colons = tag_prefix.strip_suffix("::")?;
    let namespace_end = before_colons.len();
    let namespace_start = before_colons[..namespace_end]
        .rfind(|c: char| !is_template_ident_char(c))
        .map(|index| index + 1)
        .unwrap_or(0);
    (namespace_start < namespace_end)
        .then(|| before_colons[namespace_start..namespace_end].to_string())
}

pub fn template_test_completion_requested(tag_prefix: &str) -> bool {
    let Some(test_start) = tag_prefix.rfind(" is ") else {
        return tag_prefix.contains(" is not ");
    };
    tag_prefix
        .rfind('|')
        .map(|pipe| test_start > pipe)
        .unwrap_or(true)
}

pub fn template_filter_completion_requested(tag_prefix: &str) -> bool {
    let Some(pipe) = tag_prefix.rfind('|') else {
        return false;
    };
    tag_prefix
        .rfind(" is ")
        .map(|test_start| pipe > test_start)
        .unwrap_or(true)
}

pub fn template_field_completion_path(
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

pub fn scan_template_ident_start(content: &str, lower_bound: usize, offset: usize) -> usize {
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

pub fn scan_template_ident_end(content: &str, offset: usize) -> usize {
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

pub fn is_template_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

pub fn route_completion_text(
    page: &AuthoringPage,
    current_target: &str,
    target_route: &str,
) -> String {
    if current_target.starts_with('/') {
        target_route.to_string()
    } else {
        relative_route_from_base(&page.link_base_route, target_route)
    }
}

pub fn relative_route_from_base(base_route: &str, target_route: &str) -> String {
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

pub fn route_segments(route: &str) -> Vec<&str> {
    route
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect()
}

pub fn lsp_position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
    position_to_byte_offset(content, position)
}

pub fn lsp_range_to_byte_range(content: &str, range: Range) -> Option<(usize, usize)> {
    let start = lsp_position_to_byte_offset(content, range.start)?;
    let end = lsp_position_to_byte_offset(content, range.end)?;
    (start <= end).then_some((start, end))
}

pub fn heading_id_at_position(
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
pub struct HeadingRenameTarget {
    pub heading_id: String,
    pub title: String,
    pub line: u32,
    pub title_range: Range,
}

pub fn heading_rename_target_at_position(
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
pub struct MarkdownHeading {
    pub id: String,
    pub title: String,
    pub level: u8,
    pub line: u32,
    pub title_range: Range,
}

pub fn markdown_headings(content: &str) -> Vec<MarkdownHeading> {
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

pub fn heading_title_range(content: &str, heading_byte_start: usize) -> Option<Range> {
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

pub fn frontmatter_lsp_range(content: &str) -> Option<Range> {
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
pub enum FrontmatterFieldKind {
    String,
    Integer,
    Table,
}

impl FrontmatterFieldKind {
    pub fn description(self) -> &'static str {
        match self {
            FrontmatterFieldKind::String => "a string",
            FrontmatterFieldKind::Integer => "an integer",
            FrontmatterFieldKind::Table => "a table",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrontmatterFieldSpec {
    pub name: &'static str,
    pub kind: FrontmatterFieldKind,
}

#[derive(Debug, Clone)]
pub struct FrontmatterEntry {
    pub key: String,
    pub key_start: usize,
    pub key_end: usize,
    pub value: String,
    pub value_start: usize,
    pub value_end: usize,
    pub table: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FrontmatterCompletionContext {
    pub replace_range: Range,
    pub present_fields: HashSet<String>,
    pub current_table: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateDefinitionKind {
    Block,
    Macro,
    Filter,
    Test,
}

impl TemplateDefinitionKind {
    pub fn label(self) -> &'static str {
        match self {
            TemplateDefinitionKind::Block => "block",
            TemplateDefinitionKind::Macro => "macro",
            TemplateDefinitionKind::Filter => "filter",
            TemplateDefinitionKind::Test => "test",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TemplateDefinitionTarget {
    pub kind: TemplateDefinitionKind,
    pub name: String,
    pub source_range: Range,
    pub target_path: Utf8PathBuf,
    pub target_range: Range,
}

impl TemplateDefinitionTarget {
    pub fn location(&self) -> Result<Location> {
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

    pub fn hover_markdown(&self) -> String {
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

pub fn frontmatter_field_specs() -> Vec<FrontmatterFieldSpec> {
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

pub fn frontmatter_field_kind(
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

pub fn frontmatter_entries(content: &str) -> Vec<FrontmatterEntry> {
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

pub fn frontmatter_entry_for_line(
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

pub fn frontmatter_completion_context(
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

pub fn completion_items_for_frontmatter(
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

pub fn frontmatter_document_targets(
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

pub fn frontmatter_document_kind_for_entry(
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

pub fn frontmatter_static_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .static_paths
        .get(trimmed)
        .or_else(|| project.static_paths.get(path))
}

pub fn frontmatter_data_target_path<'a>(
    project: &'a AuthoringProject,
    path: &str,
) -> Option<&'a Utf8PathBuf> {
    let trimmed = path.trim_start_matches('/');
    project
        .data_paths
        .get(trimmed)
        .or_else(|| project.data_paths.get(path))
}

pub fn frontmatter_string_value(
    content: &str,
    entry: &FrontmatterEntry,
) -> Option<(String, Range)> {
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

#[allow(deprecated)]
pub fn template_document_symbols(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Result<Vec<DocumentSymbol>> {
    if !project.template_contents.contains_key(template_file) {
        return Ok(Vec::new());
    }
    let Ok(template) = gingembre::parse_template(template_file, content) else {
        return Ok(Vec::new());
    };

    let mut symbols = Vec::new();
    collect_template_document_symbols(content, &template.body, &mut symbols);
    Ok(symbols)
}

#[allow(deprecated)]
pub fn collect_template_document_symbols(
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

impl TemplateAuthoringIndex {
    pub fn new(project: &AuthoringProject) -> Self {
        let mut templates = HashMap::new();

        for (template_file, template_path) in &project.template_paths {
            let Some(content) = project.template_contents.get(template_file) else {
                continue;
            };
            let Ok(template) = gingembre::parse_template(template_file, content) else {
                continue;
            };
            let imports = template_import_aliases(project, &template.body);
            templates.insert(
                template_file.clone(),
                IndexedTemplate {
                    path: template_path.clone(),
                    content: content.clone(),
                    semantic: project.template_semantics.get(template_file).cloned(),
                    extends: template_extends_path_from_nodes(&template.body),
                    dependencies: template_path_dependencies(&template.body),
                    diagnostics: diagnostics_for_template_nodes(
                        project,
                        template_file,
                        content,
                        &template.body,
                    ),
                    document_targets: template_document_targets_for_nodes(
                        project,
                        content,
                        &template.body,
                    ),
                    route_references: template_route_references(project, template_file, content),
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

    pub fn block_occurrence_at_position(
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

    pub fn document_targets(&self, template_file: &str) -> &[TemplateDocumentTarget] {
        self.templates
            .get(template_file)
            .map(|template| template.document_targets.as_slice())
            .unwrap_or_default()
    }

    pub fn document_target_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateDocumentTarget> {
        self.document_targets(template_file)
            .iter()
            .find(|target| range_contains_position(&target.source_range, position))
            .cloned()
    }

    pub fn route_references(&self, template_file: &str) -> &[TemplateRouteReference] {
        self.templates
            .get(template_file)
            .map(|template| template.route_references.as_slice())
            .unwrap_or_default()
    }

    pub fn route_reference_at_position(
        &self,
        template_file: &str,
        position: Position,
    ) -> Option<TemplateRouteReference> {
        self.route_references(template_file)
            .iter()
            .find(|reference| range_contains_position(&reference.source_range, position))
            .cloned()
    }

    pub fn diagnostics(&self, template_file: &str) -> &[AuthoringDiagnostic] {
        self.templates
            .get(template_file)
            .map(|template| template.diagnostics.as_slice())
            .unwrap_or_default()
    }

    pub fn all_diagnostics(&self) -> Vec<AuthoringDiagnostic> {
        let mut diagnostics = self
            .templates
            .values()
            .flat_map(|template| template.diagnostics.iter().cloned())
            .collect::<Vec<_>>();
        diagnostics.sort_by(|a, b| {
            a.source_file
                .cmp(&b.source_file)
                .then_with(|| a.byte_start.cmp(&b.byte_start))
                .then_with(|| a.target.cmp(&b.target))
        });
        diagnostics
    }

    pub fn document_reference_targets(
        &self,
        target_path: &Utf8Path,
    ) -> Vec<TemplateDocumentReferenceTarget> {
        let mut targets = Vec::new();
        for template in self.templates.values() {
            targets.extend(
                template
                    .document_targets
                    .iter()
                    .filter(|target| target.target_path == target_path)
                    .map(|target| TemplateDocumentReferenceTarget {
                        path: template.path.clone(),
                        range: target.source_range,
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

    pub fn block_definition_target(
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

    pub fn block_reference_targets(
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

    pub fn block_reference_owner(&self, template_file: &str, block_name: &str) -> Option<String> {
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

    pub fn collect_block_reference_targets(
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

    pub fn template_declares_block(&self, template_file: &str, block_name: &str) -> bool {
        self.templates
            .get(template_file)
            .is_some_and(|template| template.blocks.iter().any(|block| block.name == block_name))
    }

    pub fn macro_reference_query(
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
                source_range: occurrence.source_range,
            });
        }
        template
            .macro_calls
            .iter()
            .find(|occurrence| range_contains_position(&occurrence.source_range, position))
            .map(|occurrence| TemplateMacroReferenceQuery {
                target_template_file: occurrence.target_template_file.clone(),
                macro_name: occurrence.macro_name.clone(),
                source_range: occurrence.source_range,
            })
    }

    pub fn macro_reference_targets(
        &self,
        target_template_file: &str,
        macro_name: &str,
    ) -> Vec<TemplateMacroReferenceTarget> {
        let mut targets = Vec::new();
        if let Some(target) = self.macro_definition_target(target_template_file, macro_name) {
            targets.push(target);
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

    pub fn macro_definition_target(
        &self,
        target_template_file: &str,
        macro_name: &str,
    ) -> Option<TemplateMacroReferenceTarget> {
        let template = self.templates.get(target_template_file)?;
        template
            .macros
            .iter()
            .find(|occurrence| occurrence.name == macro_name)
            .map(|occurrence| TemplateMacroReferenceTarget {
                path: template.path.clone(),
                range: occurrence.source_range,
            })
    }

    pub fn dependency_names(&self, root_template: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        self.collect_dependency_names(root_template, &mut seen, &mut names);
        names
    }

    pub fn collect_dependency_names(
        &self,
        template_file: &str,
        seen: &mut HashSet<String>,
        names: &mut Vec<String>,
    ) {
        if !seen.insert(template_file.to_string()) {
            return;
        }
        let Some(template) = self.templates.get(template_file) else {
            return;
        };
        names.push(template_file.to_string());
        for dependency in &template.dependencies {
            self.collect_dependency_names(dependency, seen, names);
        }
    }

    pub fn semantic_tokens(&self, template_file: &str) -> Option<SemanticTokens> {
        let template = self.templates.get(template_file)?;
        let semantic = template.semantic.as_ref()?;
        Some(SemanticTokens {
            result_id: None,
            data: template_semantic_tokens(semantic, &template.content),
        })
    }

    pub fn semantic_definition(&self, template_file: &str, position: Position) -> Option<Location> {
        let template = self.templates.get(template_file)?;
        let offset = position_to_byte_offset(&template.content, position)?;
        let semantic = template.semantic.as_ref()?;
        let symbol = semantic.symbol_for_offset(offset)?;
        let span = symbol.span?;
        Some(Location {
            uri: Url::from_file_path(template.path.as_std_path()).ok()?,
            range: byte_range_to_lsp_range(
                &template.content,
                span.offset(),
                span.offset() + span.len(),
            ),
        })
    }

    pub fn semantic_references(&self, template_file: &str, position: Position) -> Vec<Location> {
        let Some(template) = self.templates.get(template_file) else {
            return Vec::new();
        };
        let Some(offset) = position_to_byte_offset(&template.content, position) else {
            return Vec::new();
        };
        let Some(semantic) = template.semantic.as_ref() else {
            return Vec::new();
        };
        let Some(symbol) = semantic.symbol_for_offset(offset) else {
            return Vec::new();
        };
        let Some(uri) = Url::from_file_path(template.path.as_std_path()).ok() else {
            return Vec::new();
        };

        let mut locations = Vec::new();
        if let Some(span) = symbol.span {
            locations.push(Location {
                uri: uri.clone(),
                range: byte_range_to_lsp_range(
                    &template.content,
                    span.offset(),
                    span.offset() + span.len(),
                ),
            });
        }
        locations.extend(
            semantic
                .references_to_symbol(symbol.id)
                .into_iter()
                .map(|reference| Location {
                    uri: uri.clone(),
                    range: byte_range_to_lsp_range(
                        &template.content,
                        reference.span.offset(),
                        reference.span.offset() + reference.span.len(),
                    ),
                }),
        );
        locations
    }
}

pub fn template_block_hover_markdown(
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

pub fn template_block_references(
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

pub fn template_block_rename_workspace_edit(
    index: &TemplateAuthoringIndex,
    template_file: &str,
    block_name: &str,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    if !is_valid_template_rename_name(new_name) {
        return Ok(None);
    }

    let mut changes = HashMap::<Url, Vec<TextEdit>>::new();
    for target in index.block_reference_targets(template_file, block_name) {
        let uri = Url::from_file_path(target.path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert template block rename path to URI: {}",
                target.path
            )
        })?;
        changes.entry(uri).or_default().push(TextEdit {
            range: target.range,
            new_text: new_name.to_string(),
        });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    for edits in changes.values_mut() {
        edits.sort_by(|left, right| {
            position_cmp(left.range.start, right.range.start)
                .then_with(|| position_cmp(left.range.end, right.range.end))
        });
        edits.dedup_by(|left, right| left.range == right.range && left.new_text == right.new_text);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

pub fn template_macro_references(
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

pub fn template_macro_rename_workspace_edit(
    index: &TemplateAuthoringIndex,
    target_template_file: &str,
    macro_name: &str,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>> {
    if !is_valid_template_rename_name(new_name) {
        return Ok(None);
    }

    let mut changes = HashMap::<Url, Vec<TextEdit>>::new();
    for target in index.macro_reference_targets(target_template_file, macro_name) {
        let uri = Url::from_file_path(target.path.as_std_path()).map_err(|_| {
            eyre!(
                "could not convert template macro rename path to URI: {}",
                target.path
            )
        })?;
        changes.entry(uri).or_default().push(TextEdit {
            range: target.range,
            new_text: new_name.to_string(),
        });
    }
    if changes.is_empty() {
        return Ok(None);
    }
    for edits in changes.values_mut() {
        edits.sort_by(|left, right| {
            position_cmp(left.range.start, right.range.start)
                .then_with(|| position_cmp(left.range.end, right.range.end))
        });
        edits.dedup_by(|left, right| left.range == right.range && left.new_text == right.new_text);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

pub fn template_definition_target_at_position(
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

pub fn template_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
) -> Result<Vec<TemplateDefinitionTarget>> {
    if !project.template_contents.contains_key(template_file) {
        return Ok(Vec::new());
    }
    let Ok(template) = gingembre::parse_template(template_file, content) else {
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

pub fn collect_template_definition_targets(
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

pub fn collect_expr_definition_targets(
    project: &AuthoringProject,
    template_file: &str,
    content: &str,
    expr: &Expr,
    imports: &HashMap<String, String>,
    targets: &mut Vec<TemplateDefinitionTarget>,
) {
    match expr {
        Expr::Optional(inner) => collect_expr_definition_targets(
            project,
            template_file,
            content,
            &inner.expr,
            imports,
            targets,
        ),
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

pub fn collect_literal_definition_targets(
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

pub fn template_block_definition_target(
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

pub fn template_block_definition(
    project: &AuthoringProject,
    template_file: &str,
    current_file: &str,
    current_content: &str,
    name: &str,
) -> Option<(String, String, Ident)> {
    let content = template_content(project, template_file, current_file, current_content)?;
    let template = gingembre::parse_template(template_file, content.as_str()).ok()?;
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

pub fn top_level_block_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Block(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

pub fn template_extends_path(
    template_file: &str,
    content: &str,
    seen: &mut HashSet<String>,
) -> Option<String> {
    if !seen.insert(template_file.to_string()) {
        return None;
    }
    let template = gingembre::parse_template(template_file, content).ok()?;
    template_extends_path_from_nodes(&template.body)
}

pub fn template_macro_definition_target(
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
    let template = gingembre::parse_template(target_file, target_content.as_str()).ok()?;
    let target_ident = top_level_macro_ident(&template.body, &macro_name.name)?;
    Some(TemplateDefinitionTarget {
        kind: TemplateDefinitionKind::Macro,
        name: format!("{}::{}", namespace.name, macro_name.name),
        source_range: template_ident_range(content, macro_name),
        target_path: project.template_paths.get(target_file).cloned()?,
        target_range: template_ident_range(&target_content, &target_ident),
    })
}

pub fn top_level_macro_ident(nodes: &[Node], name: &str) -> Option<Ident> {
    nodes.iter().find_map(|node| match node {
        Node::Macro(node) if node.name.name == name => Some(node.name.clone()),
        _ => None,
    })
}

pub fn template_content(
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

pub fn builtin_template_definition_target(
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

pub fn gingembre_eval_source_path() -> Option<Utf8PathBuf> {
    let manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest_dir.ancestors() {
        let workspace_candidate = ancestor.join("crates/gingembre/src/eval.rs");
        if workspace_candidate.exists() {
            return Some(workspace_candidate);
        }

        let crate_sibling_candidate = ancestor.join("gingembre/src/eval.rs");
        if crate_sibling_candidate.exists() {
            return Some(crate_sibling_candidate);
        }
    }
    None
}

pub fn builtin_template_definition_range(
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

pub fn frontmatter_completion_label(spec: FrontmatterFieldSpec) -> &'static str {
    match spec.kind {
        FrontmatterFieldKind::Table => "[extra]",
        _ => spec.name,
    }
}

pub fn frontmatter_completion_text(source_file: &str, spec: FrontmatterFieldSpec) -> String {
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

pub fn frontmatter_value_matches_kind(value: &str, kind: FrontmatterFieldKind) -> bool {
    let value = value.trim();
    match kind {
        FrontmatterFieldKind::String => value.starts_with('"') || value.starts_with('\''),
        FrontmatterFieldKind::Integer => frontmatter_value_is_integer(value),
        FrontmatterFieldKind::Table => true,
    }
}

pub fn frontmatter_value_is_integer(value: &str) -> bool {
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
pub struct FrontmatterContentByteRange {
    pub start: usize,
    pub end: usize,
}

pub fn frontmatter_content_byte_range(content: &str) -> Option<FrontmatterContentByteRange> {
    content.strip_prefix("+++\n")?;
    let closing_start = content[4..].find("\n+++")? + 4;
    Some(FrontmatterContentByteRange {
        start: 4,
        end: closing_start,
    })
}

pub fn frontmatter_table_name(trimmed_line: &str) -> Option<String> {
    let inner = trimmed_line.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!inner.is_empty()).then(|| inner.to_string())
}

pub fn frontmatter_table_at_offset(content: &str, offset: usize) -> Option<String> {
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

pub fn frontmatter_has_extra_table(content: &str) -> bool {
    let Some(block) = frontmatter_content_byte_range(content) else {
        return false;
    };
    content[block.start..block.end]
        .lines()
        .filter_map(|line| frontmatter_table_name(line.trim()))
        .any(|table| table == "extra")
}

pub fn is_frontmatter_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

pub fn leading_whitespace_len(input: &str) -> usize {
    input
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(input.len())
}

pub fn trailing_whitespace_len(input: &str) -> usize {
    input.len() - input.trim_end_matches(char::is_whitespace).len()
}

pub fn line_comment_start(input: &str) -> Option<usize> {
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

pub fn line_bounds_at_offset(content: &str, offset: usize) -> (usize, usize) {
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

pub fn scan_frontmatter_key_start(content: &str, line_start: usize, offset: usize) -> usize {
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

pub fn scan_frontmatter_key_end(content: &str, offset: usize, line_end: usize) -> usize {
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

pub fn source_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<String> {
    Ok(path
        .strip_prefix(content_dir)
        .map_err(|_| eyre!("content file is outside content root: {path}"))?
        .to_string())
}

pub fn template_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<Option<String>> {
    let project_dir = content_dir.parent().unwrap_or(content_dir);
    let templates_dir = project_dir.join("templates");
    match path.strip_prefix(&templates_dir) {
        Ok(relative) => Ok(logical_template_path(relative)),
        Err(_) => Ok(None),
    }
}

pub fn is_content_markdown_document(content_dir: &Utf8Path, uri: &Url) -> bool {
    lsp_file_uri_to_utf8_path(uri)
        .ok()
        .filter(|path| path.extension() == Some("md"))
        .and_then(|path| source_file_for_path(content_dir, &path).ok())
        .is_some()
}

pub fn missing_anchor_message(
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

pub fn position_to_byte_offset(content: &str, position: Position) -> Option<usize> {
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

pub fn authoring_diagnostic_to_lsp(diagnostic: &AuthoringDiagnostic) -> Diagnostic {
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

pub fn ranges_overlap(left: &Range, right: &Range) -> bool {
    position_le(left.start, right.end) && position_le(right.start, left.end)
}

pub fn source_file_for_new_route(route: &str) -> Option<String> {
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

pub fn page_frontmatter(title: &str) -> String {
    format!(
        "+++\ntitle = \"{}\"\n+++\n",
        toml_basic_string_escape(title)
    )
}

pub fn toml_basic_string_escape(input: &str) -> String {
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
pub fn create_page_command_arguments(source_uri: &Url, route: &str) -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "sourceUri": source_uri.as_str(),
        "route": route,
    })]
}

#[allow(clippy::disallowed_types)]
pub fn parse_create_page_command_arguments(
    arguments: &[serde_json::Value],
) -> Result<(Url, String)> {
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
pub fn created_page_to_json(source_file: &str, route: &str, uri: &Url) -> serde_json::Value {
    serde_json::json!({
        "sourceFile": source_file,
        "route": route,
        "uri": uri.as_str(),
    })
}

// tower-lsp command replies are JSON-RPC values; keep JSON use at this edge.
#[allow(clippy::disallowed_types)]
pub fn pages_to_json(pages: &[AuthoringPage]) -> serde_json::Value {
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
pub fn diagnostics_to_json(diagnostics: &[AuthoringDiagnostic]) -> serde_json::Value {
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
pub fn route_graph_to_json(graph: &[RouteGraphNode]) -> serde_json::Value {
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
pub fn route_graph_edges_to_json(edges: &[RouteGraphEdge]) -> serde_json::Value {
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
pub fn route_graph_edge_span_to_json(edge: &RouteGraphEdge) -> serde_json::Value {
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

pub fn diagnostic_kind_name(kind: AuthoringDiagnosticKind) -> &'static str {
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
