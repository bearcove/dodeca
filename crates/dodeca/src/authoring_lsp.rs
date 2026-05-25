use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Command, CompletionItem, CompletionItemKind,
    CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit, CreateFile,
    CreateFileOptions, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentChangeOperation, DocumentChanges, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, ExecuteCommandOptions, ExecuteCommandParams, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, Location, MarkupContent, MarkupKind,
    MessageType, NumberOrString, OneOf, OptionalVersionedTextDocumentIdentifier, Position,
    PrepareRenameResponse, Range, ReferenceParams, RenameFile, RenameFileOptions, RenameOptions,
    RenameParams, ResourceOp, ServerCapabilities, ServerInfo, ShowDocumentParams,
    SymbolInformation, SymbolKind, TextDocumentEdit, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url, WorkspaceEdit,
    WorkspaceSymbolParams,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::authoring_model::{
    AuthoringDocumentOverlay, AuthoringPage, AuthoringPageKind, AuthoringProject,
    load_authoring_project,
};
use crate::config::ResolvedConfig;
use crate::queries::default_title_from_source_path;
use crate::types::SourcePath;

const LIST_PAGES_COMMAND: &str = "dodeca.listPages";
const DIAGNOSTICS_COMMAND: &str = "dodeca.authoringDiagnostics";
const CREATE_PAGE_COMMAND: &str = "dodeca.createPage";

pub async fn run(content: Option<String>, output: Option<String>) -> Result<()> {
    let state = Arc::new(Mutex::new(AuthoringState {
        startup_args: LspStartupArgs { content, output },
        dirs: None,
        documents: HashMap::new(),
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

#[derive(Debug)]
struct AuthoringState {
    startup_args: LspStartupArgs,
    dirs: Option<AuthoringDirs>,
    documents: HashMap<Url, String>,
}

#[derive(Debug, Clone)]
struct LspStartupArgs {
    content: Option<String>,
    output: Option<String>,
}

#[derive(Debug, Clone)]
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
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
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
}

impl Backend {
    fn set_dirs(&self, dirs: AuthoringDirs) {
        self.state.lock().unwrap().dirs = Some(dirs);
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

    async fn list_pages(&self) -> Result<Vec<AuthoringPage>> {
        let dirs = self.dirs()?;
        Ok(self.current_project(&dirs).await?.pages)
    }

    async fn authoring_diagnostics(&self) -> Result<Vec<AuthoringDiagnostic>> {
        let dirs = self.dirs()?;
        let project = self.current_project(&dirs).await?;
        Ok(load_authoring_diagnostics(&project))
    }

    fn set_document(&self, uri: Url, content: String) {
        self.state.lock().unwrap().documents.insert(uri, content);
    }

    fn remove_document(&self, uri: &Url) {
        self.state.lock().unwrap().documents.remove(uri);
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

    fn document_overlays(&self, content_dir: &Utf8Path) -> Vec<AuthoringDocumentOverlay> {
        self.state
            .lock()
            .unwrap()
            .documents
            .iter()
            .filter_map(|(uri, content)| {
                let path = lsp_file_uri_to_utf8_path(uri).ok()?;
                let source_file = source_file_for_path(content_dir, &path).ok()?;
                Some(AuthoringDocumentOverlay {
                    source_file,
                    content: content.clone(),
                })
            })
            .collect()
    }

    async fn current_project(&self, dirs: &AuthoringDirs) -> Result<AuthoringProject> {
        let overlays = self.document_overlays(&dirs.content_dir);
        load_authoring_project(&dirs.content_dir, &overlays).await
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
                AuthoringDiagnosticKind::Source | AuthoringDiagnosticKind::StaticAsset => {}
            }
        }

        Ok(actions)
    }

    async fn completions(&self, params: CompletionParams) -> Result<Vec<CompletionItem>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let Some(context) = markdown_target_context_at_position(&content, position) else {
            return Ok(Vec::new());
        };

        let path = lsp_file_uri_to_utf8_path(&uri)?;
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
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            let backlink_count = references_to_page(&dirs.content_dir, &project, page)?.len();
            return Ok(Some(markdown_hover(
                frontmatter_hover_markdown(page, backlink_count),
                frontmatter_range,
            )));
        }

        let Some(reference) = reference_at_position(&content, position) else {
            return Ok(None);
        };
        let range = byte_range_to_lsp_range(&content, reference.byte_start, reference.byte_end);
        Ok(Some(markdown_hover(
            link_hover_markdown(&project, page, &content, &reference),
            range,
        )))
    }

    async fn document_symbols(&self, params: DocumentSymbolParams) -> Result<Vec<DocumentSymbol>> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let path = lsp_file_uri_to_utf8_path(&uri)?;
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
        let Some(reference) = reference_at_position(&content, position) else {
            return Ok(None);
        };

        let path = Utf8PathBuf::from_path_buf(
            uri.to_file_path()
                .map_err(|_| eyre!("LSP document URI is not a file URI: {uri}"))?,
        )
        .map_err(|path| eyre!("LSP document path is not UTF-8: {}", path.display()))?;
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        definition_for_reference(&dirs, &project, page, &reference)
    }

    async fn references_for_position(
        &self,
        uri: &Url,
        position: Position,
    ) -> Result<Vec<Location>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let path = lsp_file_uri_to_utf8_path(uri)?;
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
        let page = project
            .page_for_source_file(&source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        if let Some(frontmatter_range) = frontmatter_lsp_range(&content)
            && range_contains_position(&frontmatter_range, position)
        {
            return references_to_page(&dirs.content_dir, &project, page);
        }

        if let Some(heading_id) = heading_id_at_position(page, &content, position) {
            return references_to_heading(&dirs.content_dir, &project, page, &heading_id);
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
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
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
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let project = self.current_project(&dirs).await?;
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
                self.client.publish_diagnostics(uri, Vec::new(), None).await;
                return;
            }
        };
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringDiagnosticKind {
    Route,
    Anchor,
    Source,
    StaticAsset,
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
    markdown_references(content)
        .into_iter()
        .filter_map(|reference| diagnostic_for_reference(project, page, content, reference))
        .collect()
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
    let mut lines = vec![
        "**Dodeca page**".to_string(),
        String::new(),
        format!("Title: `{}`", page.title),
        format!("Route: `{}`", page.route),
        format!("Source: `{}`", page.source_file),
        format!("Template: `{}`", page.template),
        format!("Output: `{}`", page.output_path),
    ];

    if let Some(description) = page
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        lines.push(format!("Description: {description}"));
    }

    if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
        match project.heading_for_route(&page.route, fragment) {
            Some(heading) => lines.push(format!(
                "Heading: `{}` H{} `{}`",
                heading.id, heading.level, heading.title
            )),
            None => lines.push(format!("Heading not found: `#{fragment}`")),
        }
    }

    lines.join("\n\n")
}

fn frontmatter_hover_markdown(page: &AuthoringPage, backlink_count: usize) -> String {
    let kind = match page.kind {
        AuthoringPageKind::Page => "page",
        AuthoringPageKind::Section => "section",
    };
    let mut lines = vec![
        "**Dodeca frontmatter**".to_string(),
        String::new(),
        format!("Kind: `{kind}`"),
        format!("Title: `{}`", page.title),
        format!("Route: `{}`", page.route),
        format!("Source: `{}`", page.source_file),
        format!("Template: `{}`", page.template),
        format!("Output: `{}`", page.output_path),
        format!("Headings: `{}`", page.heading_ids.len()),
        format!("Backlinks: `{backlink_count}`"),
    ];

    if let Some(description) = page
        .description
        .as_deref()
        .filter(|description| !description.is_empty())
    {
        lines.push(format!("Description: {description}"));
    }

    lines.join("\n\n")
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

    locations.sort_by(|a, b| {
        a.uri
            .as_str()
            .cmp(b.uri.as_str())
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
    });
    Ok(locations)
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
    source_file: String,
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

    text_edits.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then_with(|| position_cmp(a.range.start, b.range.start))
            .then_with(|| position_cmp(a.range.end, b.range.end))
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
        source_file: if page.source_file == target_page.source_file {
            target.source_file.clone()
        } else {
            page.source_file.clone()
        },
        range: context.range,
        new_target,
    })
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

    let mut edits_by_source: HashMap<String, Vec<TextEdit>> = HashMap::new();
    for edit in &plan.text_edits {
        edits_by_source
            .entry(edit.source_file.clone())
            .or_default()
            .push(TextEdit::new(edit.range, edit.new_target.clone()));
    }

    let mut source_files = edits_by_source.keys().cloned().collect::<Vec<_>>();
    source_files.sort();
    for source_file in source_files {
        let uri = if source_file == plan.new_source_file {
            new_uri.clone()
        } else {
            source_file_uri(content_dir, &source_file)?
        };
        let mut edits = edits_by_source.remove(&source_file).unwrap_or_default();
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

fn source_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<String> {
    Ok(path
        .strip_prefix(content_dir)
        .map_err(|_| eyre!("content file is outside content root: {path}"))?
        .to_string())
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

fn diagnostic_kind_name(kind: AuthoringDiagnosticKind) -> &'static str {
    match kind {
        AuthoringDiagnosticKind::Route => "missingRoute",
        AuthoringDiagnosticKind::Anchor => "missingAnchor",
        AuthoringDiagnosticKind::Source => "missingSource",
        AuthoringDiagnosticKind::StaticAsset => "missingStaticAsset",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        std::fs::write(
            content_dir.join("guide/source.md"),
            "\
# Source

[ok](/guide/intro#intro)
[missing route](/missing)
[missing anchor](/guide/intro#nope)
[missing source](@/guide/missing.md)
![missing image](/missing.png)
![ok image](/logo.png)
",
        )
        .expect("write source");

        let project = load_authoring_project(&content_dir, &[])
            .await
            .expect("load project");
        let diagnostics = load_authoring_diagnostics(&project);
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
        std::fs::create_dir_all(content_dir.join("nested")).expect("create content dirs");
        std::fs::write(
            content_dir.join("target.md"),
            "+++\ntitle = \"Target\"\n+++\n\n# Target\n",
        )
        .expect("write target");
        std::fs::write(
            content_dir.join("_index.md"),
            "\
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
        let references =
            references_to_page(&content_dir, &project, target_page).expect("references");

        assert_eq!(references.len(), 4);
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
                "_index.md".to_string(),
                "nested/source.md".to_string(),
            ]
        );

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
        std::fs::create_dir_all(content_dir.join("guide")).expect("create guide dir");
        std::fs::create_dir_all(content_dir.join("nested")).expect("create nested dir");
        std::fs::write(content_dir.join("_index.md"), "# Home\n").expect("write root");
        std::fs::write(content_dir.join("guide/_index.md"), "# Guide\n").expect("write guide");
        std::fs::write(content_dir.join("nested/_index.md"), "# Nested\n").expect("write nested");
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
                .map(|edit| (edit.source_file.as_str(), edit.new_target.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("guide/source.md", "../manual/setup#intro"),
                ("guide/source.md", "../manual/setup.md#intro"),
                ("manual/setup.md", "/manual/setup#intro"),
                ("manual/setup.md", "@/manual/setup.md#intro"),
                ("nested/source.md", "/manual/setup#intro"),
                ("nested/source.md", "@/manual/setup.md#intro"),
                ("nested/source.md", "../manual/setup#intro"),
                ("nested/source.md", "../manual/setup.md#intro"),
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
        assert_eq!(operations.len(), 4);

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
        assert!(intro_hover.contains("Title: `Intro`"));
        assert!(intro_hover.contains("Heading: `intro--details` H2 `Details`"));
        assert!(intro_hover.contains("Output: `guide/intro/index.html`"));

        let missing_reference = reference_at_position(source, position_for(source, "/missing"))
            .expect("missing reference");
        let missing_hover = link_hover_markdown(&project, page, source, &missing_reference);
        assert!(missing_hover.contains("route '/missing' not found"));

        let logo_reference = reference_at_position(source, position_for(source, "/logo.png"))
            .expect("logo reference");
        let logo_hover = link_hover_markdown(&project, page, source, &logo_reference);
        assert!(logo_hover.contains("Dodeca static asset"));
        assert!(logo_hover.contains("static/logo.png"));

        let frontmatter_hover = frontmatter_hover_markdown(page, 3);
        assert!(frontmatter_hover.contains("Title: `Source`"));
        assert!(frontmatter_hover.contains("Route: `/guide/source`"));
        assert!(frontmatter_hover.contains("Template: `page.html`"));
        assert!(frontmatter_hover.contains("Backlinks: `3`"));

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
                source_file: "guide/draft.md".to_string(),
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
                source_file: "draft.md".to_string(),
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
}
