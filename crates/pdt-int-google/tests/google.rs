//! The Google plugin (PDT-316) through the API: a Google account connection, signing in (Google's consent page, then
//! its callback with a code), the file picker's token for the web app only, importing a picked Doc and Sheet, reading
//! them, the embedded editor, edits (a Doc's section, a Sheet's formulas, formats and a chart), comments, versions,
//! and signing out. Google is recorded answers, never the network.

#[path = "../../pdt-api/tests/common/mod.rs"]
mod common;

use common::{call, call_as_ui};
use pdt_api::{AppState, router};
use pdt_forge::transport::Response;
use pdt_forge::transport::testing::Recorded;
use pdt_int_google::api::{DOCS, DRIVE, SHEETS};
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
    _tmp: tempfile::TempDir,
}

async fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
    let mut st = AppState::new(store, tmp.path().join("data"));
    let google = Arc::new(Recorded::new());
    st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(pdt_int_google::Google::new(google.clone()))]));
    st.integrations.enable(&["google".into()]);
    Env { app: router(st, None), google, _tmp: tmp }
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
        {"op": "format_cells", "sheet": "Sheet1", "range": "B2:B4", "format": {"numberFormat": {"kind": "currency"}}},
        {"op": "add_chart", "sheet": "Sheet1", "chart": {"kind": "column", "title": "Costs", "domain": "A1:A3", "series": ["B1:B3"], "anchor": "D2"}}
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
