use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use camino::{Utf8Path, Utf8PathBuf};
use eyre::{Result, eyre};
use facet::Facet;
use ignore::WalkBuilder;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use tower_lsp::jsonrpc::Result as LspResult;
use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
    CodeActionProviderCapability, CodeActionResponse, Command, Diagnostic, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, ExecuteCommandOptions, ExecuteCommandParams, GotoDefinitionParams,
    GotoDefinitionResponse, InitializeParams, InitializeResult, InitializedParams, Location,
    MessageType, NumberOrString, OneOf, Position, Range, ReferenceParams, ServerCapabilities,
    ServerInfo, ShowDocumentParams, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

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
    static_dir: Utf8PathBuf,
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
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
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
    }

    // tower-lsp fixes execute-command responses to serde_json::Value at the protocol boundary.
    #[allow(clippy::disallowed_types)]
    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> LspResult<Option<serde_json::Value>> {
        match params.command.as_str() {
            LIST_PAGES_COMMAND => match self.list_pages() {
                Ok(pages) => Ok(Some(pages_to_json(&pages))),
                Err(err) => {
                    self.client
                        .log_message(MessageType::ERROR, err.to_string())
                        .await;
                    Err(tower_lsp::jsonrpc::Error::internal_error())
                }
            },
            DIAGNOSTICS_COMMAND => match self.authoring_diagnostics() {
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
        match self.code_actions(params) {
            Ok(actions) => Ok(Some(actions)),
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
        match self.definition_for_position(
            &params.text_document_position_params.text_document.uri,
            params.text_document_position_params.position,
        ) {
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
        match self.references_for_position(
            &params.text_document_position.text_document.uri,
            params.text_document_position.position,
        ) {
            Ok(locations) => Ok(Some(locations)),
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

    fn list_pages(&self) -> Result<Vec<AuthoringPage>> {
        let dirs = self.dirs()?;
        load_authoring_pages(&dirs.content_dir)
    }

    fn authoring_diagnostics(&self) -> Result<Vec<AuthoringDiagnostic>> {
        let dirs = self.dirs()?;
        load_authoring_diagnostics(&dirs.content_dir, &dirs.static_dir)
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

    fn document_overlays(&self, content_dir: &Utf8Path) -> HashMap<String, String> {
        self.state
            .lock()
            .unwrap()
            .documents
            .iter()
            .filter_map(|(uri, content)| {
                let path = lsp_file_uri_to_utf8_path(uri).ok()?;
                let source_file = source_file_for_path(content_dir, &path).ok()?;
                Some((source_file, content.clone()))
            })
            .collect()
    }

    fn code_actions(&self, params: CodeActionParams) -> Result<CodeActionResponse> {
        let uri = params.text_document.uri;
        let dirs = self.dirs_for_uri(&uri)?;
        let content = self.document_content(&uri)?;
        let diagnostics = diagnostics_for_uri(&dirs.content_dir, &dirs.static_dir, &uri, &content)?;
        let lsp_diagnostics = diagnostics
            .iter()
            .map(authoring_diagnostic_to_lsp)
            .collect::<Vec<_>>();

        let actions = diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.kind == AuthoringDiagnosticKind::Route)
            .filter(|diagnostic| ranges_overlap(&diagnostic.range(), &params.range))
            .filter_map(|diagnostic| {
                let route = diagnostic.resolved_route.as_deref()?;
                let source_file = source_file_for_new_route(route)?;
                let title = format!("Create page '{}'", source_file);
                let arguments = create_page_command_arguments(&uri, route);
                let diagnostic_range = diagnostic.range();

                Some(CodeActionOrCommand::CodeAction(CodeAction {
                    title: title.clone(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(
                        lsp_diagnostics
                            .iter()
                            .filter(|lsp_diagnostic| lsp_diagnostic.range == diagnostic_range)
                            .cloned()
                            .collect(),
                    ),
                    edit: None,
                    command: Some(Command {
                        title,
                        command: CREATE_PAGE_COMMAND.to_string(),
                        arguments: Some(arguments),
                    }),
                    is_preferred: Some(true),
                    ..CodeAction::default()
                }))
            })
            .collect();

        Ok(actions)
    }

    fn definition_for_position(&self, uri: &Url, position: Position) -> Result<Option<Location>> {
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
        let page = page_for_source_file(&source_file, &content);
        let pages = load_authoring_pages_with_overlay(&dirs.content_dir, &source_file, &content)?;
        let site_index = SiteAuthoringIndex::new(&pages);

        definition_for_reference(&dirs, &site_index, &page, &reference)
    }

    fn references_for_position(&self, uri: &Url, position: Position) -> Result<Vec<Location>> {
        let dirs = self.dirs_for_uri(uri)?;
        let content = self.document_content(uri)?;
        let Some(frontmatter_range) = frontmatter_lsp_range(&content) else {
            return Ok(Vec::new());
        };
        if !range_contains_position(&frontmatter_range, position) {
            return Ok(Vec::new());
        }

        let path = lsp_file_uri_to_utf8_path(uri)?;
        let source_file = source_file_for_path(&dirs.content_dir, &path)?;
        let mut overlays = self.document_overlays(&dirs.content_dir);
        overlays.insert(source_file.clone(), content);
        let pages = load_authoring_pages_with_overlays(&dirs.content_dir, &overlays)?;
        let site_index = SiteAuthoringIndex::new(&pages);
        let page = pages
            .iter()
            .find(|page| page.source_file == source_file)
            .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

        references_to_page(&dirs.content_dir, &site_index, &pages, page, &overlays)
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
        let diagnostics =
            match diagnostics_for_uri(&dirs.content_dir, &dirs.static_dir, &uri, &content) {
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
        let pages = match load_authoring_pages(&dirs.content_dir) {
            Ok(pages) => pages,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                return;
            }
        };
        let diagnostics = match load_authoring_diagnostics(&dirs.content_dir, &dirs.static_dir) {
            Ok(diagnostics) => diagnostics,
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, err.to_string())
                    .await;
                return;
            }
        };
        let mut diagnostics_by_source: HashMap<String, Vec<Diagnostic>> = HashMap::new();
        for diagnostic in diagnostics {
            diagnostics_by_source
                .entry(diagnostic.source_file.clone())
                .or_default()
                .push(authoring_diagnostic_to_lsp(&diagnostic));
        }

        for page in pages {
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
    let static_dir = content_dir.parent().unwrap_or(&content_dir).join("static");
    AuthoringDirs {
        content_dir,
        static_dir,
    }
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
struct AuthoringPage {
    kind: AuthoringPageKind,
    route: String,
    source_file: String,
    title: String,
    heading_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoringPageKind {
    Page,
    Section,
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

fn load_authoring_pages(content_dir: &Utf8Path) -> Result<Vec<AuthoringPage>> {
    let mut pages = Vec::new();

    for path in markdown_files(content_dir)? {
        let source_file = source_file_for_path(content_dir, &path)?;
        let source_path = SourcePath::new(source_file.clone());
        let content = std::fs::read_to_string(&path)?;
        let title = frontmatter_title(&content)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| default_title_from_source_path(&source_file));
        let heading_ids = markdown_heading_ids(&content);

        pages.push(AuthoringPage {
            kind: if source_path.is_section_index() {
                AuthoringPageKind::Section
            } else {
                AuthoringPageKind::Page
            },
            route: source_path.to_route().to_string(),
            source_file,
            title,
            heading_ids,
        });
    }

    pages.sort_by(|a, b| {
        a.route
            .cmp(&b.route)
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    Ok(pages)
}

fn load_authoring_diagnostics(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
) -> Result<Vec<AuthoringDiagnostic>> {
    let pages = load_authoring_pages(content_dir)?;
    let site_index = SiteAuthoringIndex::new(&pages);
    let mut diagnostics = Vec::new();

    for page in pages {
        let path = content_dir.join(&page.source_file);
        let content = std::fs::read_to_string(path)?;
        diagnostics.extend(diagnostics_for_page(
            content_dir,
            static_dir,
            &site_index,
            &page,
            &content,
        ));
    }

    diagnostics.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then_with(|| a.byte_start.cmp(&b.byte_start))
            .then_with(|| a.target.cmp(&b.target))
    });
    Ok(diagnostics)
}

fn diagnostics_for_uri(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
    uri: &Url,
    content: &str,
) -> Result<Vec<AuthoringDiagnostic>> {
    let path = Utf8PathBuf::from_path_buf(
        uri.to_file_path()
            .map_err(|_| eyre!("LSP document URI is not a file URI: {uri}"))?,
    )
    .map_err(|path| eyre!("LSP document path is not UTF-8: {}", path.display()))?;

    let source_file = source_file_for_path(content_dir, &path)?;
    let pages = load_authoring_pages_with_overlay(content_dir, &source_file, content)?;
    let site_index = SiteAuthoringIndex::new(&pages);
    let page = pages
        .into_iter()
        .find(|page| page.source_file == source_file)
        .ok_or_else(|| eyre!("missing authoring page for {source_file}"))?;

    Ok(diagnostics_for_page(
        content_dir,
        static_dir,
        &site_index,
        &page,
        content,
    ))
}

fn load_authoring_pages_with_overlay(
    content_dir: &Utf8Path,
    source_file: &str,
    content: &str,
) -> Result<Vec<AuthoringPage>> {
    load_authoring_pages_with_overlays(
        content_dir,
        &HashMap::from([(source_file.to_string(), content.to_string())]),
    )
}

fn load_authoring_pages_with_overlays(
    content_dir: &Utf8Path,
    overlays: &HashMap<String, String>,
) -> Result<Vec<AuthoringPage>> {
    let mut pages = load_authoring_pages(content_dir)?;

    for (source_file, content) in overlays {
        let overlay = page_for_source_file(source_file, content);
        if let Some(page) = pages
            .iter_mut()
            .find(|page| page.source_file == *source_file)
        {
            *page = overlay;
        } else {
            pages.push(overlay);
        }
    }

    pages.sort_by(|a, b| {
        a.route
            .cmp(&b.route)
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    Ok(pages)
}

fn page_for_source_file(source_file: &str, content: &str) -> AuthoringPage {
    let source_path = SourcePath::new(source_file.to_string());
    AuthoringPage {
        kind: if source_path.is_section_index() {
            AuthoringPageKind::Section
        } else {
            AuthoringPageKind::Page
        },
        route: source_path.to_route().to_string(),
        source_file: source_file.to_string(),
        title: frontmatter_title(content)
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| default_title_from_source_path(source_file)),
        heading_ids: markdown_heading_ids(content),
    }
}

#[derive(Debug)]
struct SiteAuthoringIndex {
    known_routes: HashSet<String>,
    headings_by_route: HashMap<String, HashSet<String>>,
    source_to_route: HashMap<String, String>,
    route_to_source: HashMap<String, String>,
}

impl SiteAuthoringIndex {
    fn new(pages: &[AuthoringPage]) -> Self {
        let known_routes = pages.iter().map(|page| page.route.clone()).collect();
        let headings_by_route = pages
            .iter()
            .map(|page| {
                (
                    page.route.clone(),
                    page.heading_ids.iter().cloned().collect::<HashSet<_>>(),
                )
            })
            .collect();
        let source_to_route = pages
            .iter()
            .map(|page| (page.source_file.clone(), page.route.clone()))
            .collect();
        let route_to_source = pages
            .iter()
            .map(|page| (page.route.clone(), page.source_file.clone()))
            .collect();

        Self {
            known_routes,
            headings_by_route,
            source_to_route,
            route_to_source,
        }
    }
}

fn diagnostics_for_page(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
    site_index: &SiteAuthoringIndex,
    page: &AuthoringPage,
    content: &str,
) -> Vec<AuthoringDiagnostic> {
    markdown_references(content)
        .into_iter()
        .filter_map(|reference| {
            diagnostic_for_reference(
                content_dir,
                static_dir,
                site_index,
                page,
                content,
                reference,
            )
        })
        .collect()
}

fn diagnostic_for_reference(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
    site_index: &SiteAuthoringIndex,
    page: &AuthoringPage,
    content: &str,
    reference: MarkdownReference,
) -> Option<AuthoringDiagnostic> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return None;
    }

    let (target_without_fragment, fragment) = split_fragment(target);
    let (kind, resolved_route, message) = if let Some(source_target) =
        target_without_fragment.strip_prefix("@/")
    {
        let Some(route) = site_index.source_to_route.get(source_target) else {
            return Some(reference.diagnostic(
                page,
                content,
                AuthoringDiagnosticKind::Source,
                None,
                format!("source file '{source_target}' not found"),
            ));
        };
        match missing_anchor_message(site_index, route, fragment) {
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
        if static_target_exists(
            content_dir,
            static_dir,
            &page.source_file,
            target_without_fragment,
        ) {
            return None;
        }
        (
            AuthoringDiagnosticKind::StaticAsset,
            None,
            format!("static asset '{target_without_fragment}' not found"),
        )
    } else {
        let target_route = if target_without_fragment.is_empty() {
            page.route.clone()
        } else if target_without_fragment.starts_with('/') {
            normalize_route(target_without_fragment)
        } else {
            route_for_relative_target(page, target_without_fragment)
        };

        if !route_exists(site_index, &target_route) {
            (
                AuthoringDiagnosticKind::Route,
                Some(target_route.clone()),
                format!("route '{target_route}' not found"),
            )
        } else if let Some(message) = missing_anchor_message(site_index, &target_route, fragment) {
            (AuthoringDiagnosticKind::Anchor, Some(target_route), message)
        } else {
            return None;
        }
    };

    Some(reference.diagnostic(page, content, kind, resolved_route, message))
}

fn definition_for_reference(
    dirs: &AuthoringDirs,
    site_index: &SiteAuthoringIndex,
    page: &AuthoringPage,
    reference: &MarkdownReference,
) -> Result<Option<Location>> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return Ok(None);
    }

    let (target_without_fragment, fragment) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        let path = dirs.content_dir.join(source_target);
        return location_for_source_path(&path, fragment);
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return Ok(static_target_path(
            &dirs.content_dir,
            &dirs.static_dir,
            &page.source_file,
            target_without_fragment,
        )
        .and_then(|path| location_for_path(&path, 1, 1)));
    }

    let target_route = if target_without_fragment.is_empty() {
        page.route.clone()
    } else if target_without_fragment.starts_with('/') {
        normalize_route(target_without_fragment)
    } else {
        route_for_relative_target(page, target_without_fragment)
    };

    let Some(source_file) = source_file_for_route(site_index, &target_route) else {
        return Ok(None);
    };
    let path = dirs.content_dir.join(source_file);
    location_for_source_path(&path, fragment)
}

fn references_to_page(
    content_dir: &Utf8Path,
    site_index: &SiteAuthoringIndex,
    pages: &[AuthoringPage],
    target_page: &AuthoringPage,
    overlays: &HashMap<String, String>,
) -> Result<Vec<Location>> {
    let mut locations = Vec::new();

    for page in pages {
        let content = match overlays.get(&page.source_file) {
            Some(content) => content.clone(),
            None => std::fs::read_to_string(content_dir.join(&page.source_file))?,
        };

        for reference in markdown_references(&content) {
            let Some(target_route) = reference_target_route(site_index, page, &reference) else {
                continue;
            };
            if !routes_refer_to_same_page(site_index, &target_route, &target_page.route) {
                continue;
            }
            if let Some(location) = location_for_markdown_reference(
                content_dir,
                &page.source_file,
                &content,
                &reference,
            ) {
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

fn reference_target_route(
    site_index: &SiteAuthoringIndex,
    page: &AuthoringPage,
    reference: &MarkdownReference,
) -> Option<String> {
    let target = reference.target.as_str();
    if is_special_target(target) {
        return None;
    }

    let (target_without_fragment, _) = split_fragment(target);
    if let Some(source_target) = target_without_fragment.strip_prefix("@/") {
        return site_index.source_to_route.get(source_target).cloned();
    }

    if reference.kind == MarkdownReferenceKind::Image
        || is_likely_static_file(target_without_fragment)
    {
        return None;
    }

    Some(if target_without_fragment.is_empty() {
        page.route.clone()
    } else if target_without_fragment.starts_with('/') {
        normalize_route(target_without_fragment)
    } else {
        route_for_relative_target(page, target_without_fragment)
    })
}

fn routes_refer_to_same_page(
    site_index: &SiteAuthoringIndex,
    left_route: &str,
    right_route: &str,
) -> bool {
    match (
        source_file_for_route(site_index, left_route),
        source_file_for_route(site_index, right_route),
    ) {
        (Some(left_source), Some(right_source)) => left_source == right_source,
        _ => normalize_route(left_route) == normalize_route(right_route),
    }
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

fn source_file_for_route<'a>(
    site_index: &'a SiteAuthoringIndex,
    target_route: &str,
) -> Option<&'a str> {
    site_index
        .route_to_source
        .get(target_route)
        .or_else(|| {
            site_index
                .route_to_source
                .get(target_route.trim_end_matches('/'))
        })
        .or_else(|| {
            let with_slash = format!("{}/", target_route.trim_end_matches('/'));
            site_index.route_to_source.get(&with_slash)
        })
        .map(|source_file| source_file.as_str())
}

fn route_for_relative_target(page: &AuthoringPage, target: &str) -> String {
    normalize_route(&format!("{}{target}", link_base_route(page)))
}

fn link_base_route(page: &AuthoringPage) -> String {
    if page.kind == AuthoringPageKind::Section {
        ensure_trailing_slash(&page.route)
    } else {
        let source_parent = Utf8Path::new(&page.source_file)
            .parent()
            .unwrap_or_else(|| Utf8Path::new(""));
        if source_parent.as_str().is_empty() {
            "/".to_string()
        } else {
            format!("/{}/", source_parent.as_str())
        }
    }
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
struct MarkdownHeading {
    id: String,
    line: u32,
}

fn markdown_heading_ids(content: &str) -> Vec<String> {
    markdown_headings(content)
        .into_iter()
        .map(|heading| heading.id)
        .collect()
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
                    headings.push(MarkdownHeading { id, line });
                }
            }
            _ => {}
        }
    }

    headings
}

#[derive(Debug, Default, Facet)]
struct AuthoringFrontmatter {
    #[facet(default)]
    title: Option<String>,
}

fn frontmatter_title(content: &str) -> Option<String> {
    let frontmatter = fenced_frontmatter(content)?;
    let frontmatter: AuthoringFrontmatter = facet_toml::from_str(frontmatter).ok()?;
    frontmatter.title
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

fn fenced_frontmatter(content: &str) -> Option<&str> {
    let content = content.strip_prefix("+++\n")?;
    let end = content.find("\n+++")?;
    Some(&content[..end])
}

fn markdown_files(content_dir: &Utf8Path) -> Result<Vec<Utf8PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkBuilder::new(content_dir)
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_some_and(|ty| ty.is_file()))
    {
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }

        files.push(
            Utf8PathBuf::from_path_buf(entry.into_path())
                .map_err(|path| eyre!("content path is not UTF-8: {}", path.display()))?,
        );
    }
    Ok(files)
}

fn source_file_for_path(content_dir: &Utf8Path, path: &Utf8Path) -> Result<String> {
    Ok(path
        .strip_prefix(content_dir)
        .map_err(|_| eyre!("content file is outside content root: {path}"))?
        .to_string())
}

fn route_exists(site_index: &SiteAuthoringIndex, target_route: &str) -> bool {
    site_index.known_routes.contains(target_route)
        || {
            let without_slash = target_route.trim_end_matches('/');
            !without_slash.is_empty()
                && without_slash != target_route
                && site_index.known_routes.contains(without_slash)
        }
        || {
            let with_slash = format!("{}/", target_route.trim_end_matches('/'));
            site_index.known_routes.contains(&with_slash)
        }
}

fn missing_anchor_message(
    site_index: &SiteAuthoringIndex,
    target_route: &str,
    fragment: Option<&str>,
) -> Option<String> {
    let fragment = fragment.filter(|fragment| !fragment.is_empty())?;
    let ids = site_index
        .headings_by_route
        .get(target_route)
        .or_else(|| {
            site_index
                .headings_by_route
                .get(target_route.trim_end_matches('/'))
        })
        .or_else(|| {
            let with_slash = format!("{}/", target_route.trim_end_matches('/'));
            site_index.headings_by_route.get(&with_slash)
        })?;

    if ids.contains(fragment) {
        None
    } else {
        Some(format!("anchor '#{fragment}' not found on target page"))
    }
}

fn static_target_exists(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
    source_file: &str,
    target: &str,
) -> bool {
    static_target_path(content_dir, static_dir, source_file, target).is_some()
}

fn static_target_path(
    content_dir: &Utf8Path,
    static_dir: &Utf8Path,
    source_file: &str,
    target: &str,
) -> Option<Utf8PathBuf> {
    if target.is_empty() {
        return None;
    }

    let target = strip_query(target);
    if target.starts_with('/') {
        let path = static_dir.join(target.trim_start_matches('/'));
        return path.exists().then_some(path);
    }

    let source_parent = Utf8Path::new(source_file)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let content_relative = content_dir.join(source_parent).join(target);
    if content_relative.exists() {
        return Some(content_relative);
    }

    let static_relative = static_dir.join(target);
    static_relative.exists().then_some(static_relative)
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
        assert_eq!(dirs.static_dir, dir.join("static"));

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
        assert_eq!(dirs.static_dir, project.join("static"));

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
        assert_eq!(dirs.static_dir, project.join("static"));

        std::fs::remove_dir_all(&workspace).expect("remove temp dir");
    }

    #[test]
    fn lists_pages_and_sections_from_content_dir() {
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

        let pages = load_authoring_pages(&content_dir).expect("load pages");

        assert_eq!(
            pages,
            vec![
                AuthoringPage {
                    kind: AuthoringPageKind::Section,
                    route: "/".to_string(),
                    source_file: "_index.md".to_string(),
                    title: "Knowledge Base".to_string(),
                    heading_ids: vec!["home".to_string()],
                },
                AuthoringPage {
                    kind: AuthoringPageKind::Page,
                    route: "/guide/intro".to_string(),
                    source_file: "guide/intro.md".to_string(),
                    title: "Intro".to_string(),
                    heading_ids: vec!["intro".to_string(), "intro--details".to_string()],
                },
            ]
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn reports_missing_routes_anchors_sources_and_static_assets() {
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

        let diagnostics =
            load_authoring_diagnostics(&content_dir, &static_dir).expect("load diagnostics");
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

    #[test]
    fn resolves_definition_locations_for_links_and_static_assets() {
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
            static_dir: static_dir.clone(),
        };
        let pages = load_authoring_pages(&content_dir).expect("load pages");
        let site_index = SiteAuthoringIndex::new(&pages);
        let page = pages
            .iter()
            .find(|page| page.source_file == "guide/source.md")
            .expect("source page");

        let route_reference =
            reference_at_position(source, position_for(source, "/guide/intro#intro--details"))
                .expect("route reference");
        let route_location = definition_for_reference(&dirs, &site_index, page, &route_reference)
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
        let source_location = definition_for_reference(&dirs, &site_index, page, &source_reference)
            .expect("source definition")
            .expect("source location");
        assert_eq!(source_location.range.start.line, 0);

        let relative_reference = reference_at_position(source, position_for(source, "[relative]"))
            .expect("relative reference");
        let relative_location =
            definition_for_reference(&dirs, &site_index, page, &relative_reference)
                .expect("relative definition")
                .expect("relative location");
        assert_eq!(
            relative_location.uri,
            Url::from_file_path(content_dir.join("guide/intro.md")).expect("target uri")
        );

        let static_reference = reference_at_position(source, position_for(source, "/logo.png"))
            .expect("image reference");
        let static_location = definition_for_reference(&dirs, &site_index, page, &static_reference)
            .expect("static definition")
            .expect("static location");
        assert_eq!(
            static_location.uri,
            Url::from_file_path(static_dir.join("logo.png")).expect("static uri")
        );

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn finds_references_to_page_routes_and_source_links() {
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

        let pages = load_authoring_pages(&content_dir).expect("load pages");
        let site_index = SiteAuthoringIndex::new(&pages);
        let target_page = pages
            .iter()
            .find(|page| page.source_file == "target.md")
            .expect("target page");
        let references = references_to_page(
            &content_dir,
            &site_index,
            &pages,
            target_page,
            &HashMap::new(),
        )
        .expect("references");

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
}
