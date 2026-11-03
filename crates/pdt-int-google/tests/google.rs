//! The Google plugin (PDT-316) through the API: a Google account connection, signing in (Google's consent page, then
//! its callback with a code), the file picker's token for the web app only, importing a picked Doc and Sheet, reading
//! them, the embedded editor, edits (a Doc's section, a Sheet's formulas, formats and a chart), comments, versions,
//! and signing out. Google is recorded answers, never the network.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use common::{call, call_as_ui};
use pdt_api::sdk::documents::as_agent;
use pdt_api::{AppState, router};
use pdt_forge::transport::Response;
use pdt_forge::transport::testing::Recorded;
use pdt_int_google::api::{DOCS, DRIVE, SHEETS};
use pdt_int_google::builtin::BuiltIn;
use pdt_int_google::oauth::{REVOKE_URL, TOKEN_URL, USERINFO_URL};
use pdt_store::Store;
use serde_json::{Value, json};
use std::sync::Arc;

const DOC: &str = r#"{"id":"d1","name":"Plan","mimeType":"application/vnd.google-apps.document","webViewLink":"https://docs.google.com/document/d/d1/edit","version":"7"}"#;
const SHEET: &str = r#"{"id":"s1","name":"Budget","mimeType":"application/vnd.google-apps.spreadsheet","webViewLink":"https://docs.google.com/spreadsheets/d/s1/edit","version":"3"}"#;
const TOKEN: &str = r#"{"access_token":"at1","expires_in":3600}"#;

struct Env {
    app: axum::Router,
    google: Arc<Recorded>,
    st: AppState,
    _tmp: tempfile::TempDir,
}

async fn env() -> Env {
    env_with(None).await
}

/// PDT's own clients, as a release build has them.
fn built_in() -> BuiltIn {
    BuiltIn::from_values([
        Some("std-id"),
        Some("std-secret"),
        Some("agent-id"),
        Some("agent-secret"),
        Some("pk"),
        Some("42"),
    ])
    .unwrap()
}

async fn env_with(built_in: Option<BuiltIn>) -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    let google = Arc::new(Recorded::new());
    let plugin = pdt_int_google::Google::with_built_in(google.clone(), built_in);
    st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(plugin)]));
    st.integrations.enable(&["google".into()]);
    Env { app: router(st.clone(), None), google, st, _tmp: tmp }
}

async fn account(env: &Env) -> String {
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "Work", "kind": "google",
                    "config": {"clientId": "cid.apps.googleusercontent.com", "apiKey": "AIzaKey", "appId": "1234"}})),
    )
    .await;
    assert_eq!(s, 201, "{c}");
    let id = c["id"].as_str().unwrap().to_string();
    let (s, _) =
        call(&env.app, "PUT", &format!("/api/connections/{id}/secret"), Some(json!({"secret": "csecret"}))).await;
    assert_eq!(s, 200);
    id
}

/// Signs the account in: the consent page's state, then Google's callback with a code.
async fn sign_in(env: &Env, id: &str) {
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/google/accounts/{id}/sign-in"),
        Some(json!({"origin": "http://127.0.0.1:7421"})),
    )
    .await;
    assert_eq!(s, 403, "only the web app signs in: {v}");
    let (s, v) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/google/accounts/{id}/sign-in"),
        Some(json!({"origin": "http://127.0.0.1:7421"})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["redirectUri"], "http://127.0.0.1:7421/api/google/callback");
    // The user's own client comes straight back: no website page to start at.
    assert!(v.get("relay").is_none(), "{v}");
    let url = v["url"].as_str().unwrap();
    assert!(url.contains("drive.file") && url.contains("client_id=cid.apps.googleusercontent.com"), "{url}");
    let state = url.split("state=").nth(1).unwrap().split('&').next().unwrap();
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"at0","expires_in":3600,"refresh_token":"rt1"}"#).ok(
        "GET",
        USERINFO_URL,
        r#"{"email":"me@example.com"}"#,
    );
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=abc&state={state}"), None).await;
    assert_eq!(s, 200);
    let exchange = env.google.sent().into_iter().find(|r| r.url == TOKEN_URL).unwrap();
    let body = exchange.body.unwrap();
    assert!(
        body.contains("code=abc") && body.contains("client_secret=csecret") && body.contains("code_verifier="),
        "{body}"
    );
    // The same state can't be used twice.
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=abc&state={state}"), None).await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn sign_in_pick_read_and_edit_google_docs_and_sheets() {
    let env = env().await;
    let id = account(&env).await;
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!(v[0]["signedIn"], false);
    assert_eq!((v[0]["hasClient"].as_bool(), v[0]["picker"].as_bool()), (Some(true), Some(true)));
    let (s, v) = call(&env.app, "GET", "/api/documents/providers", None).await;
    assert_eq!(s, 200);
    let google = v["providers"].as_array().unwrap().iter().find(|p| p["id"] == "google").unwrap().clone();
    assert_eq!(google["capabilities"]["embed"], true);
    // Its picker is a module of its own, which the import dialog loads (PDT-378).
    assert_eq!(google["picker"]["module"], "/api/google/picker.js");
    assert_eq!(google["picker"]["accounts"], "/api/google/accounts");
    let res = tower::ServiceExt::oneshot(
        env.app.clone(),
        axum::http::Request::get("/api/google/picker.js").body(axum::body::Body::empty()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(res.status(), 200);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/javascript"));

    sign_in(&env, &id).await;
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!((v[0]["signedIn"].as_bool(), v[0]["email"].as_str()), (Some(true), Some("me@example.com")));

    // The picker gets a token in the web app only.
    env.google.ok("POST", TOKEN_URL, TOKEN);
    let (s, _) = call(&env.app, "GET", &format!("/api/google/accounts/{id}/picker"), None).await;
    assert_eq!(s, 403);
    let (s, v) = call_as_ui(&env.app, "GET", &format!("/api/google/accounts/{id}/picker"), None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        v,
        json!({"accessToken": "at1", "apiKey": "AIzaKey", "appId": "1234", "clientId": "cid.apps.googleusercontent.com"})
    );

    // Files picked are imported by reference.
    env.google.ok("GET", &format!("{DRIVE}/files/d1?"), DOC).ok("GET", &format!("{DRIVE}/files/s1?"), SHEET);
    let (s, doc) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "google", "reference": format!("{id}/d1")})),
    )
    .await;
    assert_eq!(s, 201, "{doc}");
    assert_eq!((doc["title"].as_str(), doc["kind"].as_str()), (Some("Plan"), Some("doc")));
    let (s, sheet) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "google", "reference": format!("{id}/s1")})),
    )
    .await;
    assert_eq!(s, 201, "{sheet}");
    let (doc, sheet) = (doc["id"].as_str().unwrap().to_string(), sheet["id"].as_str().unwrap().to_string());

    // A Doc reads as Markdown, with Google's editor to embed.
    env.google.ok("GET", &format!("{DRIVE}/files/d1?"), DOC).ok(
        "GET",
        &format!("{DRIVE}/files/d1/export?mimeType=text%2Fmarkdown"),
        "# Plan\n\n## Goal\n\nShip it.\n",
    );
    let (s, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/content"), None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["content"]["markdown"], "# Plan\n\n## Goal\n\nShip it.\n");
    assert_eq!(v["embedUrl"], "https://docs.google.com/document/d/d1/edit?rm=minimal&authuser=me%40example.com");

    // A Doc's section is replaced through the Docs API.
    let structure = json!({"body": {"content": [
        {"endIndex": 1, "sectionBreak": {}},
        {"startIndex": 1, "endIndex": 6, "paragraph": {"elements": [{"textRun": {"content": "Plan\n"}}], "paragraphStyle": {"namedStyleType": "HEADING_1"}}},
        {"startIndex": 6, "endIndex": 11, "paragraph": {"elements": [{"textRun": {"content": "Goal\n"}}], "paragraphStyle": {"namedStyleType": "HEADING_2"}}},
        {"startIndex": 11, "endIndex": 20, "paragraph": {"elements": [{"textRun": {"content": "Ship it.\n"}}], "paragraphStyle": {"namedStyleType": "NORMAL_TEXT"}}}
    ]}});
    env.google
        .ok("GET", &format!("{DRIVE}/files/d1?"), DOC)
        .ok("GET", &format!("{DOCS}/documents/d1"), &structure.to_string())
        .ok("POST", &format!("{DOCS}/documents/d1:batchUpdate"), "{}")
        .ok("GET", &format!("{DRIVE}/files/d1?"), &DOC.replace("\"7\"", "\"8\""));
    let (s, v) = call(
        &env.app,
        "POST",
        &format!("/api/documents/{doc}/edits"),
        Some(json!({"edits": [{"op": "replace_section", "heading": "Goal", "markdown": "Ship it **on Friday**."}]})),
    )
    .await;
    assert_eq!((s.as_u16(), v["version"].as_str()), (200, Some("8")), "{v}");
    let update = env.google.sent().into_iter().rfind(|r| r.url.ends_with("d1:batchUpdate")).unwrap();
    let update: Value = serde_json::from_str(&update.body.unwrap()).unwrap();
    assert_eq!(update["requests"][0]["deleteContentRange"]["range"], json!({"startIndex": 11, "endIndex": 19}));

    // A Sheet: values and formulas as typed, then a format and a chart in one batch.
    env.google
        .ok("GET", &format!("{DRIVE}/files/s1?"), SHEET)
        .ok(
            "GET",
            &format!("{SHEETS}/spreadsheets/s1?fields="),
            r#"{"sheets":[{"properties":{"sheetId":0,"title":"Sheet1"}}]}"#,
        )
        .ok("POST", &format!("{SHEETS}/spreadsheets/s1/values:batchUpdate"), "{}")
        .ok("POST", &format!("{SHEETS}/spreadsheets/s1:batchUpdate"), "{}")
        .ok("GET", &format!("{DRIVE}/files/s1?"), &SHEET.replace("\"3\"", "\"4\""));
    let edits = json!({"edits": [
        {"op": "set_cells", "sheet": "Sheet1", "cell": "A1", "values": [["Month", "Cost"], ["Jan", "10"], ["Feb", "12"], ["Total", "=SUM(B2:B3)"]]},
        {"op": "format_range", "sheet": "Sheet1", "range": "B2:B4", "format": {"numberFormat": "$#,##0.00"}},
        {"op": "add_chart", "sheet": "Sheet1", "id": "costs", "chart": {"kind": "column", "title": "Costs", "data": "A1:B3", "anchor": "D2"}}
    ]});
    let (s, v) = call(&env.app, "POST", &format!("/api/documents/{sheet}/edits"), Some(edits)).await;
    assert_eq!((s.as_u16(), v["version"].as_str()), (200, Some("4")), "{v}");
    let sent = env.google.sent();
    let values: Value = serde_json::from_str(
        sent.iter().rfind(|r| r.url.contains("values:batchUpdate")).unwrap().body.as_deref().unwrap(),
    )
    .unwrap();
    assert_eq!(values["data"][0]["values"][3][1], "=SUM(B2:B3)");
    let batch: Value = serde_json::from_str(
        sent.iter().rfind(|r| r.url.ends_with("s1:batchUpdate")).unwrap().body.as_deref().unwrap(),
    )
    .unwrap();
    assert!(batch["requests"][0]["repeatCell"].is_object() && batch["requests"][1]["addChart"].is_object(), "{batch}");
    // The chart keeps the id PDT gave it.
    assert_eq!(v["charts"], json!(["costs"]));
    assert!(
        batch["requests"][2]["createDeveloperMetadata"]["developerMetadata"]["metadataValue"]
            .as_str()
            .unwrap()
            .ends_with(" costs")
    );

    // A Sheet reads as cells with formulas.
    env.google.ok("GET", &format!("{DRIVE}/files/s1?"), SHEET).ok(
        "GET",
        &format!("{SHEETS}/spreadsheets/s1?includeGridData=true"),
        r#"{"sheets":[{"properties":{"title":"Sheet1"},"data":[{"rowData":[{"values":[{"formattedValue":"Total"},{"formattedValue":"$22.00","userEnteredValue":{"formulaValue":"=SUM(B2:B3)"}}]}]}]}]}"#,
    );
    let (_, v) = call(&env.app, "GET", &format!("/api/documents/{sheet}/content"), None).await;
    assert_eq!(v["content"]["sheets"][0]["rows"][0][1], json!({"value": "$22.00", "formula": "=SUM(B2:B3)"}));

    // Comments: listed, replied to and resolved through Drive.
    env.google
        .ok("GET", &format!("{DRIVE}/files/d1/comments"), r#"{"comments":[{"id":"k1","author":{"displayName":"Sam"},"content":"Friday?","resolved":false,"replies":[]}]}"#)
        .ok("POST", &format!("{DRIVE}/files/d1/comments/k1/replies"), r#"{"id":"r1","author":{"displayName":"Me"},"content":"Yes"}"#)
        .ok("POST", &format!("{DRIVE}/files/d1/comments/k1/replies"), r#"{"id":"r2"}"#);
    let (s, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/comments"), None).await;
    assert_eq!((s.as_u16(), v[0]["body"].as_str()), (200, Some("Friday?")), "{v}");
    let (s, v) =
        call(&env.app, "POST", &format!("/api/documents/{doc}/comments/k1/replies"), Some(json!({"body": "Yes"})))
            .await;
    assert_eq!((s.as_u16(), v["id"].as_str()), (201, Some("r1")), "{v}");
    let (s, _) = call(&env.app, "POST", &format!("/api/documents/{doc}/comments/k1/resolve"), None).await;
    assert!(s.is_success(), "{s}");

    // Its version, for open pages to refresh on.
    env.google.ok("GET", &format!("{DRIVE}/files/d1?"), &DOC.replace("\"7\"", "\"9\""));
    let (_, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None).await;
    assert_eq!(v["version"], "9");

    // A rate limit says so.
    env.google.on(
        "GET",
        &format!("{DRIVE}/files/d1?"),
        Response::new(429, r#"{"error":{"code":429,"message":"Quota exceeded","status":"RESOURCE_EXHAUSTED"}}"#),
    );
    let (s, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None).await;
    assert_eq!((s.as_u16(), v["code"].as_str()), (429, Some("provider_busy")), "{v}");

    // Signing out takes PDT's access back.
    env.google.ok("POST", REVOKE_URL, "{}");
    let (s, _) = call_as_ui(&env.app, "DELETE", &format!("/api/google/accounts/{id}/sign-in"), None).await;
    assert_eq!(s, 204);
    assert!(env.google.urls().iter().any(|u| u.starts_with(&format!("POST {REVOKE_URL}?token=rt1"))));
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!(v[0]["signedIn"], false);
    let (s, v) = call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None).await;
    assert_eq!(s, 502);
    assert!(v["message"].as_str().unwrap().contains("isn't signed in"), "{v}");
}

#[tokio::test]
async fn a_connection_needs_its_client_id_and_both_picker_settings() {
    let env = env().await;
    let (s, v) =
        call(&env.app, "POST", "/api/connections", Some(json!({"name": "X", "kind": "google", "config": {}}))).await;
    assert_eq!(s, 400, "{v}");
    let (s, _) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "X", "kind": "google", "config": {"clientId": "c", "apiKey": "k"}})),
    )
    .await;
    assert_eq!(s, 400);
    let (s, v) = call(&env.app, "GET", "/api/documents/providers/google/files", None).await;
    assert_eq!(s, 502);
    assert!(v["message"].as_str().unwrap().contains("signed in"), "{v}");
}

/// Deleting a Google account takes its sign-in with it: Google gets the refresh token back and the keychain forgets
/// it, with the plugin turned on or off.
#[tokio::test]
async fn deleting_an_account_revokes_and_forgets_its_sign_in() {
    for on in [true, false] {
        let env = env().await;
        let id = account(&env).await;
        sign_in(&env, &id).await;
        let key = pdt_int_google::accounts::sign_in_key(&id);
        assert!(env.st.secrets.get(&key).unwrap().is_some());
        if !on {
            let (s, _) = call(&env.app, "PUT", "/api/integrations", Some(json!({"enabled": []}))).await;
            assert_eq!(s, 200);
        }
        env.google.ok("POST", REVOKE_URL, "{}");
        let (s, _) = call(&env.app, "DELETE", &format!("/api/connections/{id}"), None).await;
        assert_eq!(s, 204, "plugin on: {on}");
        assert!(env.google.urls().iter().any(|u| u.starts_with(&format!("POST {REVOKE_URL}?token=rt1"))), "on: {on}");
        assert_eq!(env.st.secrets.get(&key).unwrap(), None, "on: {on}");
        assert_eq!(env.st.secrets.get(&format!("connection:{id}")).unwrap(), None);
    }
}

/// Google can't be reached: the account is deleted all the same, and PDT forgets its sign-in.
#[tokio::test]
async fn an_account_is_deleted_even_when_google_cant_take_its_sign_in_back() {
    let env = env().await;
    let id = account(&env).await;
    sign_in(&env, &id).await;
    // No answer recorded for the revoke: Google says 404.
    let (s, _) = call(&env.app, "DELETE", &format!("/api/connections/{id}"), None).await;
    assert_eq!(s, 204);
    assert_eq!(env.st.secrets.get(&pdt_int_google::accounts::sign_in_key(&id)).unwrap(), None);
}

/// The consent page's `state`, from its URL.
fn state_of(url: &str) -> String {
    url.split("state=").nth(1).unwrap().split('&').next().unwrap().to_string()
}

/// The last token exchange's body.
fn exchange_body(env: &Env) -> String {
    env.google.sent().into_iter().rfind(|r| r.url == TOKEN_URL).unwrap().body.unwrap()
}

/// A build with PDT's own clients but no picker key (a missing release secret): its accounts never ask for a key,
/// while an account with its own client still can be given one (PDT-493).
#[tokio::test]
async fn a_build_without_the_picker_key_says_so_instead_of_asking_for_one() {
    let b = BuiltIn::from_values([
        Some("std-id"),
        Some("std-secret"),
        Some("agent-id"),
        Some("agent-secret"),
        None,
        Some("42"),
    ]);
    let env = env_with(b).await;
    let (s, c) =
        call(&env.app, "POST", "/api/connections", Some(json!({"name": "Mine", "kind": "google", "config": {}}))).await;
    assert_eq!(s, 201, "{c}");
    let (s, c) = call(
        &env.app,
        "POST",
        "/api/connections",
        Some(json!({"name": "Org", "kind": "google", "config": {"clientId": "cid.apps.googleusercontent.com"}})),
    )
    .await;
    assert_eq!(s, 201, "{c}");
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    let of = |name: &str| v.as_array().unwrap().iter().find(|a| a["name"] == name).unwrap().clone();
    let (mine, org) = (of("Mine"), of("Org"));
    assert_eq!((&mine["builtIn"], &mine["picker"]), (&json!(true), &json!(false)));
    assert_eq!(mine["note"], "No file picker in this build of PDT");
    assert_eq!((&org["builtIn"], &org["picker"]), (&json!(false), &json!(false)));
    assert_eq!(org["note"], "No file picker (needs an API key and project number)");

    // The import dialog's texts for each: the help asks only to sign in, a built-in account hears the build has no
    // picker, an own-client account how to add one, and the empty search what drive.file finds.
    let (_, v) = call(&env.app, "GET", "/api/documents/providers", None).await;
    let google = v["providers"].as_array().unwrap().iter().find(|p| p["id"] == "google").unwrap().clone();
    let picker = &google["picker"];
    let text = |k: &str| picker[k].as_str().unwrap().to_string();
    assert!(!text("help").contains("API key"), "{picker}");
    assert!(text("unavailable").contains("This build of PDT doesn't include Google's file picker"), "{picker}");
    assert!(!text("unavailable").contains("API key"), "{picker}");
    assert!(text("setup").contains("picker API key and project number"), "{picker}");
    assert!(text("searchNote").contains("opened in PDT"), "{picker}");
}

/// With PDT's own clients: Sign in with Google adds the account (back through the website's page, from a Tailscale
/// address), the picker gets PDT's key, agents need a permission of their own with the agentic client, never the
/// user's token, and taking it away forgets only the agents' sign-in.
#[tokio::test]
async fn sign_in_with_pdts_own_clients_and_a_separate_permission_for_agents() {
    let env = env_with(Some(built_in())).await;
    // The plugin declares its sign-in, with PDT's own client (PDT-378).
    let (_, v) = call(&env.app, "GET", "/api/integrations", None).await;
    let google = v.as_array().unwrap().iter().find(|p| p["id"] == "google").unwrap();
    assert_eq!((&google["signIn"]["api"], &google["signIn"]["builtIn"]), (&json!("/api/google"), &json!(true)));

    // Only the web app, and only from an address that looks like a PDT.
    let from = json!({"origin": "http://100.101.102.103:7420"});
    let (s, _) = call(&env.app, "POST", "/api/google/sign-in", Some(from.clone())).await;
    assert_eq!(s, 403);
    let (s, v) =
        call_as_ui(&env.app, "POST", "/api/google/sign-in", Some(json!({"origin": "https://evil.example"}))).await;
    assert_eq!(s, 400, "{v}");
    let (s, v) = call_as_ui(&env.app, "POST", "/api/google/sign-in", Some(from)).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["redirectUri"], "https://pdt.dev/oauth/google");
    // The browser starts there too, so the page knows which PDT started the sign-in.
    assert_eq!(v["relay"], "https://pdt.dev/oauth/google");
    let url = v["url"].as_str().unwrap();
    assert!(url.contains("client_id=std-id") && url.contains("code_challenge="), "{url}");
    let state = state_of(url);
    // The website's page reads where to go back to from the state.
    let (nonce, origin) = state.split_once('.').unwrap();
    assert_eq!(origin, "aHR0cDovLzEwMC4xMDEuMTAyLjEwMzo3NDIw");

    // A state PDT didn't issue (the same address, another nonce) is refused, and the real one still works after.
    let forged = format!("{}x.{origin}", &nonce[1..]);
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=abc&state={forged}"), None).await;
    assert_eq!(s, 400);
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"u0","expires_in":3600,"refresh_token":"user-rt"}"#).ok(
        "GET",
        USERINFO_URL,
        r#"{"email":"me@example.com"}"#,
    );
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=abc&state={state}"), None).await;
    assert_eq!(s, 200);
    let body = exchange_body(&env);
    for part in [
        "client_id=std-id",
        "client_secret=std-secret",
        "code_verifier=",
        "redirect_uri=https%3A%2F%2Fpdt.dev%2Foauth%2Fgoogle",
    ] {
        assert!(body.contains(part), "{part} in {body}");
    }
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    let id = v[0]["connectionId"].as_str().unwrap().to_string();
    assert_eq!(v[0]["name"], "me@example.com", "named after the account: {v}");
    assert_eq!(
        (&v[0]["signedIn"], &v[0]["builtIn"], &v[0]["hasClient"], &v[0]["agents"], &v[0]["picker"]),
        (&json!(true), &json!(true), &json!(true), &json!(false), &json!(true))
    );
    let (_, conns) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(conns.as_array().unwrap().len(), 1);

    // The picker has PDT's key and project.
    env.google.ok("POST", TOKEN_URL, TOKEN);
    let (s, v) = call_as_ui(&env.app, "GET", &format!("/api/google/accounts/{id}/picker"), None).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((&v["apiKey"], &v["appId"], &v["clientId"]), (&json!("pk"), &json!("42"), &json!("std-id")));

    // The user's own reads use the user's token.
    env.google.ok("GET", &format!("{DRIVE}/files/d1?"), DOC);
    let (s, doc) = call(
        &env.app,
        "POST",
        "/api/documents/import",
        Some(json!({"provider": "google", "reference": format!("{id}/d1")})),
    )
    .await;
    assert_eq!(s, 201, "{doc}");
    let doc = doc["id"].as_str().unwrap().to_string();

    // An agent may not, until the user allows agents: and never with the user's token.
    let before = env.google.sent().len();
    let (s, v) = as_agent(call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None)).await;
    assert_eq!(s, 502);
    assert!(v["message"].as_str().unwrap().contains("Allow agents"), "{v}");
    assert_eq!(env.google.sent().len(), before, "nothing went to Google");
    let (s, v) = as_agent(call(&env.app, "GET", "/api/documents/providers/google/files", None)).await;
    assert_eq!(s, 502);
    assert!(v["message"].as_str().unwrap().contains("lets agents"), "{v}");

    // Allowing agents: a consent of its own, with the agentic client, as the same account.
    let (s, v) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/google/accounts/{id}/sign-in"),
        Some(json!({"origin": "https://mac.tail1.ts.net", "agent": true})),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let url = v["url"].as_str().unwrap();
    assert!(url.contains("client_id=agent-id") && url.contains("login_hint=me%40example.com"), "{url}");
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"x","expires_in":3600,"refresh_token":"other-rt"}"#).ok(
        "GET",
        USERINFO_URL,
        r#"{"email":"someone-else@example.com"}"#,
    );
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=c2&state={}", state_of(url)), None).await;
    assert_eq!(s, 400, "another Google account can't be the agents'");
    let (_, v) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/google/accounts/{id}/sign-in"),
        Some(json!({"origin": "https://mac.tail1.ts.net", "agent": true})),
    )
    .await;
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"a0","expires_in":3600,"refresh_token":"agent-rt"}"#).ok(
        "GET",
        USERINFO_URL,
        r#"{"email":"Me@example.com"}"#,
    );
    let state = state_of(v["url"].as_str().unwrap());
    let (s, _) = call(&env.app, "GET", &format!("/api/google/callback?code=c3&state={state}"), None).await;
    assert_eq!(s, 200);
    assert!(exchange_body(&env).contains("client_secret=agent-secret"));
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!(v[0]["agents"], true);

    // An agent's call uses the agents' token, from their refresh token and the agentic client.
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"agent-at","expires_in":3600}"#).ok(
        "GET",
        &format!("{DRIVE}/files/d1?"),
        &DOC.replace("\"7\"", "\"8\""),
    );
    let (s, v) = as_agent(call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None)).await;
    assert_eq!((s.as_u16(), v["version"].as_str()), (200, Some("8")), "{v}");
    let sent = env.google.sent();
    let refresh = sent.iter().rfind(|r| r.url == TOKEN_URL).unwrap().body.clone().unwrap();
    assert!(refresh.contains("refresh_token=agent-rt") && refresh.contains("client_id=agent-id"), "{refresh}");
    let get = sent.last().unwrap();
    assert!(get.headers.iter().any(|(k, v)| *k == "authorization" && v == "Bearer agent-at"));

    // The user's calls still use the user's token.
    env.google.ok("GET", &format!("{DRIVE}/files/d1?"), DOC);
    call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None).await;
    let get = env.google.sent().pop().unwrap();
    assert!(get.headers.iter().any(|(k, v)| *k == "authorization" && v == "Bearer at1"), "{:?}", get.headers);

    // Taking the agents' permission away forgets their sign-in, without asking Google (which would sign the user out).
    let (s, _) = call(&env.app, "DELETE", &format!("/api/google/accounts/{id}/agents"), None).await;
    assert_eq!(s, 403);
    let (s, _) = call_as_ui(&env.app, "DELETE", &format!("/api/google/accounts/{id}/agents"), None).await;
    assert_eq!(s, 204);
    assert_eq!(env.st.secrets.get(&pdt_int_google::accounts::agent_key(&id)).unwrap(), None);
    assert!(env.st.secrets.get(&pdt_int_google::accounts::sign_in_key(&id)).unwrap().is_some());
    assert!(!env.google.urls().iter().any(|u| u.contains(REVOKE_URL)));
    let (s, v) = as_agent(call(&env.app, "GET", &format!("/api/documents/{doc}/version"), None)).await;
    assert_eq!(s, 502, "{v}");
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!((&v[0]["signedIn"], &v[0]["agents"]), (&json!(true), &json!(false)));

    // Signing in again as the same account keeps it one account.
    let (_, v) =
        call_as_ui(&env.app, "POST", "/api/google/sign-in", Some(json!({"origin": "http://localhost:7421"}))).await;
    env.google.ok("POST", TOKEN_URL, r#"{"access_token":"u1","expires_in":3600,"refresh_token":"user-rt2"}"#).ok(
        "GET",
        USERINFO_URL,
        r#"{"email":"me@example.com"}"#,
    );
    let (s, _) = call(
        &env.app,
        "GET",
        &format!("/api/google/callback?code=c4&state={}", state_of(v["url"].as_str().unwrap())),
        None,
    )
    .await;
    assert_eq!(s, 200);
    let (_, conns) = call(&env.app, "GET", "/api/connections", None).await;
    assert_eq!(conns.as_array().unwrap().len(), 1);
}

/// With its own OAuth client an account keeps signing in as before, through PDT's own callback, and agents use its
/// sign-in; it can't ask for the agents' permission.
#[tokio::test]
async fn an_account_with_its_own_client_works_beside_pdts() {
    let env = env_with(Some(built_in())).await;
    let id = account(&env).await;
    sign_in(&env, &id).await;
    let (_, v) = call(&env.app, "GET", "/api/google/accounts", None).await;
    assert_eq!((&v[0]["builtIn"], &v[0]["agents"]), (&json!(false), &json!(true)));
    let (s, _) = call_as_ui(
        &env.app,
        "POST",
        &format!("/api/google/accounts/{id}/sign-in"),
        Some(json!({"origin": "http://127.0.0.1:7421", "agent": true})),
    )
    .await;
    assert_eq!(s, 400);
    // A connection with no client of its own is PDT's.
    let (s, v) =
        call(&env.app, "POST", "/api/connections", Some(json!({"name": "X", "kind": "google", "config": {}}))).await;
    assert_eq!(s, 201, "{v}");
}

/// A build without PDT's clients offers only the user's own: no Sign in with Google for a new account.
#[tokio::test]
async fn without_pdts_clients_only_your_own() {
    let env = env().await;
    let (_, v) = call(&env.app, "GET", "/api/integrations", None).await;
    let google = v.as_array().unwrap().iter().find(|p| p["id"] == "google").unwrap();
    assert_eq!(google["signIn"]["builtIn"], false);
    let (s, v) =
        call_as_ui(&env.app, "POST", "/api/google/sign-in", Some(json!({"origin": "http://127.0.0.1:7420"}))).await;
    assert_eq!(s, 400, "{v}");
}
