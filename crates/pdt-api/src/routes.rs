use crate::error::ApiError;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use pdt_core::brand::PRODUCT_NAME;
use pdt_core::project::{NewProject, Project};
use pdt_store::Store;
use serde::Serialize;
use std::path::PathBuf;
use tower_http::services::{ServeDir, ServeFile};
use ts_rs::TS;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
}

#[derive(Debug, Serialize, TS)]
#[ts(export)]
pub struct Health {
    pub status: String,
    pub name: String,
    pub version: String,
}

pub fn router(state: AppState, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", get(get_project))
        .fallback(|| async { ApiError::not_found("route") })
        .method_not_allowed_fallback(|| async {
            ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed", "method not allowed")
        })
        .with_state(state);
    let app = Router::new().nest("/api", api);
    match web_dir {
        Some(dir) => {
            let index = dir.join("index.html");
            app.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)))
        }
        None => app,
    }
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok".into(), name: PRODUCT_NAME.into(), version: env!("CARGO_PKG_VERSION").into() })
}

async fn list_projects(State(st): State<AppState>) -> Result<Json<Vec<Project>>, ApiError> {
    Ok(Json(st.store.list_projects().await?))
}

async fn get_project(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Project>, ApiError> {
    st.store.get_project(&id).await?.map(Json).ok_or_else(|| ApiError::not_found("project"))
}

async fn create_project(
    State(st): State<AppState>,
    payload: Result<Json<NewProject>, JsonRejection>,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let Json(input) = payload.map_err(|rej| ApiError::new(rej.status(), "invalid_input", rej.body_text()))?;
    let valid = input.validate()?;
    let path = PathBuf::from(&valid.repo_path);
    let info =
        tokio::task::spawn_blocking(move || pdt_git::resolve_repo(&path)).await.map_err(ApiError::internal)??;
    let project = Project {
        id: uuid::Uuid::now_v7().to_string(),
        name: valid.name,
        repo_path: info.root.to_string_lossy().into_owned(),
        base_branch: valid.base_branch.or(info.current_branch).unwrap_or_else(|| "main".into()),
        created_at: jiff::Timestamp::now().as_millisecond(),
    };
    st.store.insert_project(&project).await?;
    Ok((StatusCode::CREATED, Json(project)))
}
