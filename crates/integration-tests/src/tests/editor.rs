//! In-browser editor SAVE, end-to-end.
//!
//! These tests drive the same vox-over-websocket path the browser editor uses
//! (`crates/dodeca-devtools/src/state.rs::connect_websocket`), against a `ddc
//! serve --dev-editor` instance whose source is a git repo tracking a bare
//! `origin`. They prove the "fetch before push" fix in
//! `crates/dodeca/src/serve.rs` (commit 7a76f69): `commit_as_user` now fetches
//! origin and rebases the editor's commit onto the upstream before pushing, so a
//! save still succeeds when `origin/main` moved between load and save — which a
//! bare `git push` would reject as non-fast-forward.

use super::*;
use crate::harness::run_git;

use dodeca_protocol::{
    BrowserService, BrowserServiceDispatcher, DevtoolsEvent, DevtoolsServiceClient, EditLoad,
    EditSave, EditSaveReq, EditTokenResponse,
};
use std::path::Path;
use vox::FromVoxSession;
use vox_websocket::WsLink;

/// Local handler for the reverse-direction `BrowserService` (the server pushes
/// devtools events to the browser). The editor save flow doesn't depend on these
/// events, so we just trace and drop them — but we must register a handler, like
/// the real browser does, or the session's connection acceptor has nothing to
/// dispatch host-initiated calls to.
#[derive(Clone)]
struct NoopBrowserService;

impl BrowserService for NoopBrowserService {
    async fn on_event(&self, event: DevtoolsEvent) {
        tracing::debug!(?event, "editor test: ignoring browser event");
    }
}

/// A connected native editor client, holding the vox root alive for the
/// session's lifetime (dropping `_root` tears the websocket down).
struct EditorClient {
    client: DevtoolsServiceClient,
    _root: vox::NoopClient,
}

/// Connect to `/_/ws` and open a `DevtoolsService` virtual connection, mirroring
/// the browser's `connect_websocket` exactly: a bare initiator session with a
/// `BrowserService` handler, then a service vconn carrying the
/// `vox-service = DevtoolsService` metadata, driven by a spawned `Driver`.
async fn connect_editor(port: u16) -> EditorClient {
    let url = format!("ws://127.0.0.1:{port}/_/ws");
    let link = WsLink::connect(&url)
        .await
        .unwrap_or_else(|e| panic!("websocket connect {url}: {e}"));

    let dispatcher = BrowserServiceDispatcher::new(NoopBrowserService);
    let root = vox::initiator_on(link, vox::TransportMode::Bare)
        .on_connection(dispatcher)
        .establish::<vox::NoopClient>()
        .await
        .unwrap_or_else(|e| panic!("vox root handshake: {e:?}"));

    let session = root
        .session
        .clone()
        .expect("vox root session handle missing");
    let settings = vox::ConnectionSettings {
        parity: vox::Parity::Odd,
        max_concurrent_requests: 64,
        initial_channel_credit: 16,
    };
    let handle = session
        .open_connection(
            settings,
            vec![vox::MetadataEntry::str(
                vox::VOX_SERVICE_METADATA_KEY,
                DevtoolsServiceClient::SERVICE_NAME,
            )],
        )
        .await
        .unwrap_or_else(|e| panic!("open DevtoolsService connection: {e:?}"));

    let mut driver = vox::Driver::new(handle, BrowserServiceDispatcher::new(NoopBrowserService));
    let client = DevtoolsServiceClient::from_vox_session(vox::Caller::new(driver.caller()), None);
    tokio::spawn(async move { driver.run().await });

    EditorClient {
        client,
        _root: root,
    }
}

/// Fetch the editor session token from the well-known endpoint, parsed with
/// facet-json (serde is banned).
async fn edit_token(site: &TestSite) -> String {
    let resp = site.get("/_dodeca/edit-token").await;
    resp.assert_ok();
    resp.assert_content_type("application/json");
    let parsed: EditTokenResponse = facet_json::from_str(resp.text())
        .unwrap_or_else(|e| panic!("parse edit-token JSON {:?}: {e:?}", resp.text()));
    assert!(!parsed.token.is_empty(), "edit token must be non-empty");
    parsed.token
}

/// Destructure `EditLoad::Ok` or fail with the variant we got.
fn expect_load_ok(load: EditLoad) -> (String, String, String) {
    match load {
        EditLoad::Ok {
            source_key,
            content,
            base,
            ..
        } => (source_key, content, base),
        other => panic!("expected EditLoad::Ok, got {other:?}"),
    }
}

/// Clone the bare `origin` into `<fixture>/.clone-<name>` and return its path.
/// Used both to push a concurrent commit and to verify origin's final state.
fn clone_origin(fixture_dir: &Path, name: &str) -> std::path::PathBuf {
    let origin = fixture_dir.join(".origin.git");
    let dest = fixture_dir.join(format!(".clone-{name}"));
    // Clone `main` explicitly: the bare origin's HEAD follows git's
    // `init.defaultBranch`, which is `main` on some machines and `master` on
    // others (CI). A bare `git clone` would then check out a nonexistent default
    // branch and leave an empty working tree (no `content/`), so name the branch.
    run_git(
        fixture_dir,
        &[
            "clone",
            "-b",
            "main",
            origin.to_str().expect("utf8 origin path"),
            dest.to_str().expect("utf8 clone path"),
        ],
    );
    dest
}

/// THE FIX: a concurrent push advances `origin/main` between the editor's load
/// and its save. Before 7a76f69 the save's bare `git push` was rejected
/// (non-fast-forward); now `commit_as_user` fetches + rebases first, so the save
/// succeeds AND the concurrent commit is preserved in origin.
pub async fn moved_remote_save_succeeds() {
    let site = TestSite::with_editor_repo("editor-site", "editor");
    // Make sure the server is fully up and serving the page before we edit it.
    site.wait_until("page renders", Duration::from_secs(30), async || {
        let resp = site.get("/page/").await;
        (resp.status == 200).then_some(())
    })
    .await;

    let token = edit_token(&site).await;
    let editor = connect_editor(site.port).await;

    // Load the page for editing — captures the source_key + the base blob oid the
    // save will be checked against.
    let load = editor
        .client
        .edit_load(token.clone(), "/page".to_string())
        .await
        .expect("edit_load RPC");
    let (source_key, _content, base) = expect_load_ok(load);

    // Concurrent change: a SECOND clone of origin commits a DIFFERENT file and
    // pushes, advancing origin/main past the served checkout. This is exactly
    // what the /_dodeca/pull webhook or another editor tab would do.
    let other = clone_origin(site.fixture_dir(), "concurrent");
    fs_err::write(
        other.join("content").join("other.md"),
        "+++\ntitle = \"Other\"\n+++\n\nconcurrent edit\n",
    )
    .expect("write concurrent file");
    run_git(&other, &["add", "-A"]);
    run_git(
        &other,
        &[
            "-c",
            "user.email=other@localhost",
            "-c",
            "user.name=other",
            "commit",
            "-m",
            "concurrent: add other.md",
        ],
    );
    run_git(&other, &["push", "origin", "main"]);

    // Now save the editor's edit to /page. origin/main has moved, so the old
    // bare push would fail "fetch first"; the fix fetches + rebases first.
    let new_body = "+++\ntitle = \"Editable Page\"\n+++\n\n# Editable Page\n\nEdited via the in-browser editor.\n";
    let save = editor
        .client
        .edit_save(
            token.clone(),
            EditSaveReq {
                source_key: source_key.clone(),
                buffer: new_body.to_string(),
                base: base.clone(),
                message: "edit: update page".to_string(),
            },
        )
        .await
        .expect("edit_save RPC");

    match &save {
        EditSave::Ok { commit, .. } => {
            assert!(!commit.is_empty(), "saved commit hash must be non-empty");
        }
        other => panic!("expected EditSave::Ok after concurrent push, got {other:?}"),
    }

    // Verify origin now contains BOTH commits: clone it afresh and check that the
    // editor's edit landed AND the concurrent file is still present (the rebase
    // preserved it rather than clobbering it).
    let verify = clone_origin(site.fixture_dir(), "verify");
    let page = fs_err::read_to_string(verify.join("content").join("page.md"))
        .expect("read page.md from origin clone");
    assert!(
        page.contains("Edited via the in-browser editor."),
        "origin's page.md should contain the editor's edit, got:\n{page}"
    );
    assert!(
        verify.join("content").join("other.md").exists(),
        "origin should still contain the concurrent commit's other.md"
    );

    // And the editor's commit is authored as the --dev-editor user.
    let log = run_git(&verify, &["log", "--format=%an <%ae> %s", "-n", "5"]);
    assert!(
        log.contains("editor <editor@localhost> edit: update page"),
        "origin log should show the editor's authored commit, got:\n{log}"
    );
    assert!(
        log.contains("concurrent: add other.md"),
        "origin log should still show the concurrent commit, got:\n{log}"
    );
}

/// The per-viewer Edit button is gated on `can_edit`, which dodeca threads into
/// the gingembre render context as a tracked argument keyed per viewer. The
/// `editor-site` page template emits a `data-n-edit` button only inside
/// `{% if can_edit %}`. This is the oracle that `can_edit` actually reaches the
/// template AND differs by viewer.
///
/// Editor view: spawned with `--dev-editor`, so the synthesized identity passes
/// the same gate `mint_edit_token` uses → `can_edit = true` → the button renders.
pub async fn edit_button_visible_to_editor() {
    let site = TestSite::with_editor_repo("editor-site", "editor");
    site.wait_until("page renders", Duration::from_secs(30), async || {
        let resp = site.get("/page/").await;
        (resp.status == 200).then_some(())
    })
    .await;

    let resp = site.get("/page/").await;
    resp.assert_ok();
    // The HTML cell re-serializes the valueless attribute as `data-n-edit=""`,
    // so assert on that canonical form.
    resp.assert_contains(r#"<button data-n-edit="">Edit</button>"#);
}

/// Anonymous view: the SAME fixture spawned WITHOUT `--dev-editor` and with no
/// `auth` config → `can_edit = false` → the Edit button is absent. Proves the
/// gate is real (not always-on) and that the anonymous render omits it.
pub async fn edit_button_hidden_from_anonymous() {
    let site = TestSite::new("editor-site");
    site.wait_until("page renders", Duration::from_secs(30), async || {
        let resp = site.get("/page/").await;
        (resp.status == 200).then_some(())
    })
    .await;

    let resp = site.get("/page/").await;
    resp.assert_ok();
    resp.assert_not_contains("data-n-edit");
    resp.assert_not_contains(">Edit<");
}

/// A genuine optimistic-concurrency conflict: the on-disk file changed since the
/// editor loaded it, so the `base`-vs-disk check fails *before* committing and
/// `edit_save` returns `EditSave::Conflict` (read `edit_save` in serve.rs). We
/// drive this by passing a stale `base` that no longer matches the file's blob.
pub async fn stale_base_reports_conflict() {
    let site = TestSite::with_editor_repo("editor-site", "editor");
    site.wait_until("page renders", Duration::from_secs(30), async || {
        let resp = site.get("/page/").await;
        (resp.status == 200).then_some(())
    })
    .await;

    let token = edit_token(&site).await;
    let editor = connect_editor(site.port).await;

    let load = editor
        .client
        .edit_load(token.clone(), "/page".to_string())
        .await
        .expect("edit_load RPC");
    let (source_key, _content, base) = expect_load_ok(load);
    assert!(
        !base.is_empty(),
        "page.md exists on disk, so its base blob oid must be non-empty"
    );

    // A stale, non-matching base: the optimistic check compares it against the
    // file's current blob oid (unchanged on disk), they differ → Conflict, and we
    // never reach the commit/push path.
    let stale_base = "0000000000000000000000000000000000000000".to_string();
    let save = editor
        .client
        .edit_save(
            token.clone(),
            EditSaveReq {
                source_key,
                buffer: "+++\ntitle = \"Editable Page\"\n+++\n\nclobber attempt\n".to_string(),
                base: stale_base,
                message: "edit: should conflict".to_string(),
            },
        )
        .await
        .expect("edit_save RPC");

    match save {
        EditSave::Conflict { current } => {
            assert_eq!(
                current, base,
                "Conflict's `current` should be the file's actual on-disk blob oid"
            );
        }
        other => panic!("expected EditSave::Conflict for a stale base, got {other:?}"),
    }
}
