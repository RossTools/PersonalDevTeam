//! Documents (PDT-314) through the API with a fake provider: create, import, browse, filter, link to projects and
//! tickets, rename, read, edit, comments, delete; a ticket's design doc; the settings that pick the provider and
//! folder; and `get_ticket` listing a ticket's documents (`fake-agent` stands in for Claude and calls pdt-mcp tools
//! when a message says `mcp: <tool> <json>`).

use crate::common;

use axum::http::StatusCode;
use common::{call, git};
use pdt_api::documents::DocumentProviders;
use pdt_api::documents::testing::FakeProvider;
use pdt_api::{AppState, router};
use pdt_core::document::DocumentKind;
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-agent");

struct Env {
    app: axum::Router,
    state: AppState,
    project: String,
    fake: Arc<FakeProvider>,
    other: Arc<FakeProvider>,
    tmp: tempfile::TempDir,
}

/// A project with a repo and two fake providers (`fake`, and `other`, which can only list and read), served on a free
/// local port (the agent's pdt-mcp calls come over TCP).
async fn setup() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-m", "init", "-q"]);
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut state = AppState::new(store, tmp.path().join("data"));
    state.platform.claude_program = FAKE.to_string();
    state.platform.login_path = None;
    state.platform.claude_config_dir = Some(tmp.path().join("claude-config"));
    state.platform.codex_home = Some(tmp.path().join("codex-home"));
    state.home_dir = Some(tmp.path().join("home"));
    state.conversations.notice_quiet = Duration::ZERO;
    let fake = Arc::new(FakeProvider::new("fake", "Fake"));
    let other = Arc::new(FakeProvider::new("other", "Other"));
    *other.capabilities.lock().unwrap() =
        pdt_core::document::ProviderCapabilities { docs: true, sheets: true, ..Default::default() };
    state.documents = DocumentProviders::new(vec![fake.clone(), other.clone()]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    state.listen_port = listener.local_addr().unwrap().port();
    let app = router(state.clone(), None);
    let served = app.clone();
    tokio::spawn(async move { axum::serve(listener, served).await });
    let (s, p) =
        call(&app, "POST", "/api/projects", Some(json!({"name": "P", "repoPath": repo.to_string_lossy()}))).await;
    assert_eq!(s, StatusCode::CREATED, "{p}");
    Env { app, state, project: p["id"].as_str().unwrap().into(), fake, other, tmp }
}

async fn ticket(env: &Env, title: &str) -> Value {
    let (s, t) =
        call(&env.app, "POST", &format!("/api/projects/{}/tickets", env.project), Some(json!({"title": title}))).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    t
}

async fn create(env: &Env, body: Value) -> Value {
    let (s, d) = call(&env.app, "POST", "/api/documents", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    d
}

async fn titles(env: &Env, query: &str) -> Vec<String> {
    let (s, list) = call(&env.app, "GET", &format!("/api/documents{query}"), None).await;
    assert_eq!(s, StatusCode::OK, "{list}");
    list.as_array().unwrap().iter().map(|d| d["title"].as_str().unwrap().to_string()).collect()
}

#[tokio::test]
async fn documents_are_created_browsed_filtered_and_linked() {
    let env = setup().await;
    let t = ticket(&env, "Faster boards").await;
    let (tid, key) = (t["id"].as_str().unwrap(), t["key"].as_str().unwrap());

    // The providers, with the first as the default and PDT's data folder for local files.
    let (_, p) = call(&env.app, "GET", "/api/documents/providers", None).await;
    assert_eq!(p["providers"].as_array().unwrap().len(), 2, "{p}");
    assert_eq!(p["default"], "fake");
    assert_eq!(p["folder"], env.state.data_dir.join("documents").to_string_lossy().as_ref());

    let plan = create(&env, json!({"title": " Plan ", "kind": "doc", "projectId": env.project, "by": "user"})).await;
    assert_eq!((&plan["title"], &plan["provider"], &plan["actor"]), (&json!("Plan"), &json!("fake"), &json!("user")));
    assert_eq!(plan["projects"][0]["id"], env.project.as_str());
    // A ticket (by key) links its project too; no `by` is an agent.
    let budget = create(&env, json!({"title": "Budget", "kind": "sheet", "ticketId": key})).await;
    assert_eq!(budget["actor"], "agent");
    assert_eq!(budget["tickets"][0]["key"], key);
    assert_eq!(budget["projects"][0]["id"], env.project.as_str());
    assert_eq!(env.fake.folders.lock().unwrap()[0], env.state.data_dir.join("documents"));

    // Browse and filter.
    assert_eq!(titles(&env, "").await, ["Budget", "Plan"]);
    assert_eq!(titles(&env, &format!("?ticket={tid}")).await, ["Budget"]);
    assert_eq!(titles(&env, &format!("?ticket={key}")).await, ["Budget"]);
    assert_eq!(titles(&env, &format!("?project={}", env.project)).await, ["Budget", "Plan"]);
    assert_eq!(titles(&env, "?kind=doc").await, ["Plan"]);
    assert_eq!(titles(&env, "?provider=other").await, Vec::<String>::new());
    assert_eq!(titles(&env, "?q=pla").await, ["Plan"]);

    // Link the plan to the ticket, then unlink it.
    let plan_id = plan["id"].as_str().unwrap();
    let (s, d) =
        call(&env.app, "POST", &format!("/api/documents/{plan_id}/links"), Some(json!({"ticketId": tid}))).await;
    assert_eq!(s, StatusCode::OK, "{d}");
    assert_eq!(titles(&env, &format!("?ticket={tid}")).await, ["Plan", "Budget"]);
    let (s, d) =
        call(&env.app, "DELETE", &format!("/api/documents/{plan_id}/links"), Some(json!({"ticketId": tid}))).await;
    assert_eq!((s, d["tickets"].as_array().unwrap().len()), (StatusCode::OK, 0), "{d}");
    let (s, e) = call(&env.app, "POST", &format!("/api/documents/{plan_id}/links"), Some(json!({}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{e}");
    let (s, _) =
        call(&env.app, "POST", &format!("/api/documents/{plan_id}/links"), Some(json!({"ticketId": "X-9"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    // Rename: the provider's file too.
    let (s, d) = call(&env.app, "PATCH", &format!("/api/documents/{plan_id}"), Some(json!({"title": "Q4 plan"}))).await;
    assert_eq!((s, &d["title"]), (StatusCode::OK, &json!("Q4 plan")));
    let (_, f) = call(&env.app, "GET", "/api/documents/providers/fake/files?q=q4", None).await;
    assert_eq!(f[0]["title"], "Q4 plan", "{f}");
    let (s, _) = call(&env.app, "PATCH", &format!("/api/documents/{plan_id}"), Some(json!({"title": " "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // A provider that can't make documents refuses; one PDT doesn't have is named.
    let (s, e) =
        call(&env.app, "POST", "/api/documents", Some(json!({"title": "X", "kind": "doc", "provider": "other"}))).await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("unsupported")), "{e}");
    let (s, e) =
        call(&env.app, "POST", "/api/documents", Some(json!({"title": "X", "kind": "doc", "provider": "gone"}))).await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("no_provider")), "{e}");
}

#[tokio::test]
async fn files_are_imported_once_and_read_edited_and_commented_on() {
    let env = setup().await;
    let reference = env.fake.put("Roadmap", DocumentKind::Doc, "# Roadmap\n");
    env.other.put("Theirs", DocumentKind::Sheet, "");

    let (_, found) = call(&env.app, "GET", "/api/documents/providers/fake/files?q=road", None).await;
    assert_eq!(found[0]["reference"], reference.as_str(), "{found}");
    let (s, e) = call(&env.app, "GET", "/api/documents/providers/other/files", None).await;
    assert_eq!((s, &e["code"]), (StatusCode::BAD_REQUEST, &json!("unsupported")), "{e}");

    let body = json!({"provider": "fake", "reference": reference, "projectId": env.project, "by": "user"});
    let (s, d) = call(&env.app, "POST", "/api/documents/import", Some(body.clone())).await;
    assert_eq!((s, &d["title"]), (StatusCode::CREATED, &json!("Roadmap")), "{d}");
    let (s, again) = call(&env.app, "POST", "/api/documents/import", Some(body)).await;
    assert_eq!((s, &again["id"]), (StatusCode::OK, &d["id"]), "imported once");
    let (s, _) =
        call(&env.app, "POST", "/api/documents/import", Some(json!({"provider": "fake", "reference": "nope"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let id = d["id"].as_str().unwrap();

    let (_, r) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(r["content"], json!({"kind": "doc", "markdown": "# Roadmap\n", "version": "1"}), "{r}");
    let edits = json!({"edits": [{"op": "append_text", "markdown": "More."}]});
    let (s, v) = call(&env.app, "POST", &format!("/api/documents/{id}/edits"), Some(edits)).await;
    assert_eq!((s, &v["version"]), (StatusCode::OK, &json!("2")), "{v}");
    let (_, r) = call(&env.app, "GET", &format!("/api/documents/{id}/content"), None).await;
    assert_eq!(r["content"]["markdown"], "# Roadmap\nMore.");
    let bad = json!({"edits": [{"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [["1"]]}]});
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{id}/edits"), Some(bad)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/comments"), None).await;
    assert_eq!(c[0]["body"], "Is this still right?", "{c}");
    let (s, r) =
        call(&env.app, "POST", &format!("/api/documents/{id}/comments/c1/replies"), Some(json!({"body": "Yes."})))
            .await;
    assert_eq!((s, &r["body"]), (StatusCode::CREATED, &json!("Yes.")));
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{id}/comments/c1/resolve"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, c) = call(&env.app, "GET", &format!("/api/documents/{id}/comments"), None).await;
    assert_eq!((&c[0]["resolved"], &c[0]["replies"][0]["body"]), (&json!(true), &json!("Yes.")));

    // A sheet from a provider that can only read: no edits, no comments, no delete of its file.
    let theirs = env.other.put("Theirs 2", DocumentKind::Sheet, "");
    let (_, s2) =
        call(&env.app, "POST", "/api/documents/import", Some(json!({"provider": "other", "reference": theirs}))).await;
    let sid = s2["id"].as_str().unwrap();
    let (_, r) = call(&env.app, "GET", &format!("/api/documents/{sid}/content"), None).await;
    assert_eq!(r["content"]["sheets"][0]["name"], "Sheet1", "{r}");
    let (s, _) = call(&env.app, "GET", &format!("/api/documents/{sid}/comments"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/documents/{sid}?file=true"), None).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "DELETE", &format!("/api/documents/{sid}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "PDT forgets it; the file stays");
    let (_, f) = call(&env.app, "GET", "/api/documents/providers/fake/files", None).await;
    assert_eq!(f.as_array().unwrap().len(), 1);

    // Deleting with the file deletes it at the provider too.
    let (s, _) = call(&env.app, "DELETE", &format!("/api/documents/{id}?file=true"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, f) = call(&env.app, "GET", "/api/documents/providers/fake/files", None).await;
    assert_eq!(f.as_array().unwrap().len(), 0);
    let (s, _) = call(&env.app, "GET", &format!("/api/documents/{id}"), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn settings_pick_the_provider_and_folder_of_new_documents() {
    let env = setup().await;
    let (s, h) = call(
        &env.app,
        "PUT",
        "/api/host/settings",
        Some(json!({"documentsProvider": "other", "documentsDir": "~/Docs"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{h}");
    let (_, p) = call(&env.app, "GET", "/api/documents/providers", None).await;
    assert_eq!(p["default"], "other");
    assert_eq!(p["folder"], env.tmp.path().join("home").join("Docs").to_string_lossy().as_ref());

    let own = env.tmp.path().join("own-docs").to_string_lossy().into_owned();
    let uri = format!("/api/projects/{}/document-settings", env.project);
    let (s, saved) = call(&env.app, "PUT", &uri, Some(json!({"provider": "fake", "folder": own}))).await;
    assert_eq!(s, StatusCode::OK, "{saved}");
    let (_, p) = call(&env.app, "GET", &format!("/api/documents/providers?project={}", env.project), None).await;
    assert_eq!((&p["default"], &p["folder"]), (&json!("fake"), &json!(own)));
    let d = create(&env, json!({"title": "Here", "kind": "doc", "projectId": env.project})).await;
    assert_eq!(d["provider"], "fake");
    assert_eq!(env.fake.folders.lock().unwrap().last().unwrap().to_string_lossy(), own);
    let (s, _) = call(&env.app, "PUT", &uri, Some(json!({"provider": "Bad id", "folder": null}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = call(&env.app, "PUT", &uri, Some(json!({"provider": null, "folder": "relative"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

async fn agent_says(env: &Env, session: &str) -> Vec<String> {
    let (_, v) = call(&env.app, "GET", &format!("/api/sessions/{session}/events?after=0"), None).await;
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["event"]["kind"] == "agent_message")
        .map(|e| e["event"]["markdown"].as_str().unwrap().to_string())
        .collect()
}

/// Sends the agent `text` and waits for its answer to the tool call it makes.
async fn say(env: &Env, session: &str, text: &str) -> String {
    let before = agent_says(env, session).await.len();
    let (s, v) =
        call(&env.app, "POST", &format!("/api/sessions/{session}/messages"), Some(json!({"text": text}))).await;
    assert_eq!(s, StatusCode::ACCEPTED, "{v}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let said = agent_says(env, session).await;
        if let Some(m) = said.iter().skip(before).find(|m| m.starts_with("mcp ")) {
            return m.clone();
        }
        assert!(Instant::now() < deadline, "the agent said only {said:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_tickets_design_doc_is_made_linked_and_listed_by_get_ticket() {
    let env = setup().await;
    let t = ticket(&env, "Faster boards").await;
    let (id, key) = (t["id"].as_str().unwrap(), t["key"].as_str().unwrap());

    let (s, d) = call(&env.app, "POST", &format!("/api/tickets/{key}/design-doc"), Some(json!({"by": "user"}))).await;
    assert_eq!(s, StatusCode::CREATED, "{d}");
    assert_eq!(d["title"], format!("{key} design: Faster boards"));
    assert_eq!((&d["kind"], &d["provider"]), (&json!("doc"), &json!("fake")));
    assert_eq!((&d["tickets"][0]["id"], &d["projects"][0]["id"]), (&json!(id), &json!(env.project)));
    let did = d["id"].as_str().unwrap();
    let (_, r) = call(&env.app, "GET", &format!("/api/documents/{did}/content"), None).await;
    let md = r["content"]["markdown"].as_str().unwrap();
    for h in ["## Goal", "## Scope", "## Out of scope", "## Done when", "## Notes"] {
        assert!(md.contains(h), "{md}");
    }
    let (s, _) = call(&env.app, "POST", "/api/tickets/X-404/design-doc", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);

    let (s, started) = call(
        &env.app,
        "POST",
        &format!("/api/tickets/{id}/start"),
        Some(json!({"agent": "claude", "sessionMode": "conversation"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{started}");
    let session = started["session"]["id"].as_str().unwrap().to_string();
    let said = say(&env, &session, "mcp: get_ticket {}").await;
    let got: Value = serde_json::from_str(said.strip_prefix("mcp get_ticket: ").unwrap_or(&said))
        .unwrap_or_else(|e| panic!("{e}: {said}"));
    let docs = got["documents"].as_array().unwrap_or_else(|| panic!("{got:#}"));
    assert_eq!(docs.len(), 1);
    assert_eq!(
        (&docs[0]["id"], &docs[0]["title"], &docs[0]["kind"], &docs[0]["provider"]),
        (&json!(did), &d["title"], &json!("doc"), &json!("fake"))
    );
    assert!(got["documentsNote"].as_str().unwrap().contains("design doc"), "{got:#}");
}
