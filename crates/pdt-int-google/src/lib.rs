//! Google (PDT-316): Google Docs and Sheets as PDT documents. Each Google account is a `google` connection, signed in
//! with the user's own OAuth client and only the `drive.file` scope: PDT reaches the files it makes and those the user
//! picks in Google's file picker, nothing else in their Drive. Docs read as Markdown and change through the Docs API;
//! Sheets read as cells with their formulas and change through the Sheets API (values, formulas, formats, conditional
//! formats, validation, charts, sorting and filters); comments through Drive. The document page embeds Google's own
//! editor. pdtd's `google` feature; on in Settings → Plugins.

pub mod accounts;
pub mod api;
pub mod docs;
pub mod drive;
pub mod oauth;
pub mod provider;
pub mod routes;
pub mod sheets;

use accounts::{KIND, Shared};
use pdt_api::sdk::documents::DocumentProvider;
use pdt_api::sdk::{AppState, Integration};
use pdt_core::integration::{ConfigField, ConnectionKind};
use pdt_forge::transport::Transport;
use serde_json::Value;
use std::sync::Arc;

pub struct Google {
    shared: Arc<Shared>,
}

impl Google {
    /// The plugin, reaching Google through `transport` (recorded answers in tests).
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { shared: Arc::new(Shared::new(transport)) }
    }
}

impl Integration for Google {
    fn id(&self) -> &'static str {
        "google"
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: KIND.into(),
            label: "Google account".into(),
            plural: "Google accounts".into(),
            setup: "One per Google account, with your own OAuth client from Google Cloud. Save it, then Sign in with \
                    Google below. PDT sees only the files it makes and those you pick."
                .into(),
            description: "A Google account for Docs and Sheets (drive.file: only files PDT makes or you pick).".into(),
            fields: vec![
                ConfigField {
                    placeholder: Some("1234-abc.apps.googleusercontent.com".into()),
                    hint: Some("The OAuth client's ID (Google Cloud → APIs & Services → Credentials).".into()),
                    ..ConfigField::new("clientId", "OAuth client ID")
                },
                ConfigField {
                    placeholder: Some("AIza…".into()),
                    hint: Some(
                        "An API key of the same Google Cloud project, for Google's file picker. Without it you \
                         can't pick files from your Drive."
                            .into(),
                    ),
                    optional: true,
                    ..ConfigField::new("apiKey", "Picker API key")
                },
                ConfigField {
                    placeholder: Some("123456789012".into()),
                    hint: Some(
                        "The Google Cloud project's number (its dashboard shows it), for the file picker.".into(),
                    ),
                    optional: true,
                    ..ConfigField::new("appId", "Project number")
                },
                ConfigField {
                    placeholder: Some("https://drive.google.com/drive/folders/…".into()),
                    hint: Some("Where new Docs and Sheets go: a Drive folder's address or ID. Empty: My Drive.".into()),
                    optional: true,
                    ..ConfigField::new("folder", "Folder for new documents")
                },
            ],
            secret_label: "OAuth client secret".into(),
            secret_hint: "The OAuth client's secret, shown with its ID in Google Cloud.".into(),
            ..Default::default()
        }]
    }

    fn check_connection_config(&self, _kind: &str, config: &Value) -> Result<(), String> {
        let s = |k: &str| config.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        if s("clientId").is_none() {
            return Err("a Google account needs its OAuth client ID".into());
        }
        if let Some(f) = s("folder")
            && drive::folder_id(f).is_none()
        {
            return Err(format!("{f} is not a Drive folder's address or ID"));
        }
        if s("apiKey").is_some() != s("appId").is_some() {
            return Err("Google's file picker needs both the API key and the project number".into());
        }
        Ok(())
    }

    /// A deleted account is signed out: Google takes the refresh token back and PDT forgets it.
    fn connection_deleted<'a>(
        &'a self,
        st: &'a AppState,
        connection: &'a pdt_core::workflow::api::Connection,
    ) -> pdt_api::sdk::BoxFut<'a, ()> {
        Box::pin(async move {
            if let Err(e) = accounts::sign_out(st, &self.shared, &connection.id).await {
                tracing::warn!(connection = connection.name, error = %e, "a deleted Google account's sign-in stayed");
            }
        })
    }

    fn document_providers(&self, st: &AppState) -> Vec<Arc<dyn DocumentProvider>> {
        vec![Arc::new(provider::GoogleDocs { st: st.clone(), shared: self.shared.clone() })]
    }

    fn routes(&self) -> Option<axum::Router<AppState>> {
        use axum::routing::{get, post};
        Some(
            axum::Router::new()
                .route("/google/accounts", get(routes::list))
                .route("/google/accounts/{id}/sign-in", post(routes::start).delete(routes::sign_out))
                .route("/google/accounts/{id}/picker", get(routes::picker))
                .route("/google/callback", get(routes::callback))
                .layer(axum::Extension(self.shared.clone())),
        )
    }
}
