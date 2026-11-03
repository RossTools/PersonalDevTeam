//! Personas: versioned agent setups agent steps run. Every save is a new version; built-ins
//! (Refiner, Coder, Reviewer, QA) are installed on start and follow newer templates until the user edits them.

use crate::error::{ApiError, OrNotFound, codes};
use crate::routes::{AppState, now_ms};
use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use pdt_core::persona::{
    BUILTIN_PERSONAS, NewPersona, Persona, PersonaAuthor, PersonaDiff, PersonaPatch, PersonaVersion, SavePersona,
    diff as persona_diff, valid_name,
};
use serde::Deserialize;

async fn load(st: &AppState, id: &str) -> Result<Persona, ApiError> {
    st.store.get_persona(id).await?.or_404("persona")
}

/// `GET /api/personas/tools` (PDT-110): every pdt-mcp tool a persona may pick, with what it does.
pub(crate) async fn tools(State(st): State<AppState>) -> Json<Vec<pdt_core::persona::PersonaTool>> {
    Json(st.registry().tool_table().persona_tools())
}

pub(crate) async fn list(State(st): State<AppState>) -> Result<Json<Vec<Persona>>, ApiError> {
    ensure_builtins(&st).await;
    Ok(Json(st.store.list_personas().await?))
}

pub(crate) async fn create(
    State(st): State<AppState>,
    payload: Result<Json<NewPersona>, JsonRejection>,
) -> Result<(StatusCode, Json<Persona>), ApiError> {
    let Json(n) = payload?;
    let name = valid_name(&n.name).map_err(ApiError::invalid)?;
    let setup = n.setup.validate(st.registry().tool_table()).map_err(ApiError::invalid)?;
    known_agent(&st, setup.agent).await?;
    let now = now_ms();
    let p = Persona {
        id: uuid::Uuid::now_v7().to_string(),
        name,
        role: n.role.trim().to_string(),
        description: n.description.trim().to_string(),
        builtin: None,
        archived: false,
        version: 1,
        setup,
        created_at: now,
        updated_at: now,
    };
    st.store.insert_persona(&p, PersonaAuthor::User, "created").await?;
    Ok((StatusCode::CREATED, Json(load(&st, &p.id).await?)))
}

/// A persona's ACP agent is one this host has (a built-in agent always is).
async fn known_agent(st: &AppState, agent: pdt_core::session::AgentKind) -> Result<(), ApiError> {
    if agent.is_acp() {
        crate::conversations::acp_agent(st, agent).await?;
    }
    Ok(())
}

pub(crate) async fn get_one(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Persona>, ApiError> {
    Ok(Json(load(&st, &id).await?))
}

/// `PUT /api/personas/{id}`: a new version.
pub(crate) async fn save(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<SavePersona>, JsonRejection>,
) -> Result<Json<Persona>, ApiError> {
    let Json(s) = payload?;
    let current = load(&st, &id).await?;
    let setup = s.setup.validate(st.registry().tool_table()).map_err(ApiError::invalid)?;
    if setup.agent != current.setup.agent {
        known_agent(&st, setup.agent).await?;
    }
    if setup == current.setup {
        return Ok(Json(current));
    }
    st.store.save_persona_version(&id, &setup, PersonaAuthor::User, s.note.trim(), now_ms()).await?;
    Ok(Json(load(&st, &id).await?))
}

pub(crate) async fn patch(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: Result<Json<PersonaPatch>, JsonRejection>,
) -> Result<Json<Persona>, ApiError> {
    let Json(p) = payload?;
    let mut current = load(&st, &id).await?;
    if let Some(n) = p.name {
        current.name = valid_name(&n).map_err(ApiError::invalid)?;
    }
    if let Some(r) = p.role {
        current.role = r.trim().to_string();
    }
    if let Some(d) = p.description {
        current.description = d.trim().to_string();
    }
    if let Some(a) = p.archived {
        current.archived = a;
    }
    st.store.update_persona(&current, now_ms()).await?;
    Ok(Json(load(&st, &id).await?))
}

/// The workflows whose agent steps name a persona (by name or id).
async fn used_by(st: &AppState, p: &Persona) -> Result<Vec<String>, ApiError> {
    let mut out = vec![];
    for w in st.store.list_workflows(None).await? {
        let Some(doc) = st.store.workflow_doc(&w.id, w.version).await? else { continue };
        let names = doc.nodes.iter().filter(|n| n.kind == crate::workflow::nodes::agent::KIND).any(|n| {
            n.config
                .get("persona")
                .and_then(|v| v.as_str())
                .is_some_and(|x| x == p.id || x.eq_ignore_ascii_case(&p.name))
        });
        if names {
            out.push(w.name);
        }
    }
    Ok(out)
}

pub(crate) async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let p = load(&st, &id).await?;
    if p.builtin.is_some() {
        return Err(ApiError::conflict(codes::BUILTIN_PERSONA, format!("{} is built in: archive it instead", p.name)));
    }
    let used = used_by(&st, &p).await?;
    if !used.is_empty() {
        return Err(ApiError::conflict(
            codes::PERSONA_IN_USE,
            format!("{} runs in {}: change those workflows first, or archive it", p.name, used.join(", ")),
        ));
    }
    st.store.delete_persona(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn versions(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<PersonaVersion>>, ApiError> {
    load(&st, &id).await?;
    Ok(Json(st.store.list_persona_versions(&id).await?))
}

pub(crate) async fn version(
    State(st): State<AppState>,
    Path((id, v)): Path<(String, u32)>,
) -> Result<Json<PersonaVersion>, ApiError> {
    st.store.get_persona_version(&id, v).await?.map(Json).or_404("persona version")
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DiffQuery {
    from: Option<u32>,
    to: Option<u32>,
}

/// `GET /api/personas/{id}/diff?from=&to=`: by default the current version against the one before.
pub(crate) async fn diff(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<PersonaDiff>, ApiError> {
    let p = load(&st, &id).await?;
    let to = q.to.unwrap_or(p.version);
    let from = q.from.unwrap_or(to.saturating_sub(1).max(1));
    let a = st.store.get_persona_version(&id, from).await?.or_404("persona version")?;
    let b = st.store.get_persona_version(&id, to).await?.or_404("persona version")?;
    Ok(Json(persona_diff(&a.setup, &b.setup, &format!("v{from}"), &format!("v{to}"), st.registry().tool_table())))
}

/// `GET /api/personas/{id}/template` (PDT-100): a built-in persona's current template, for "Load PDT's template".
pub(crate) async fn template(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<pdt_core::persona::PersonaTemplate>, ApiError> {
    let p = load(&st, &id).await?;
    let b = p.builtin.as_deref().and_then(pdt_core::persona::builtin_persona).ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, codes::NOT_BUILTIN, format!("{} is not a built-in persona", p.name))
    })?;
    let setup = b.setup(st.registry().tool_table());
    let diff = persona_diff(&p.setup, &setup, &format!("v{}", p.version), "PDT's template", st.registry().tool_table());
    Ok(Json(pdt_core::persona::PersonaTemplate { setup, diff }))
}

/// Installs the built-in personas, or brings them up to newer templates: fields the user changed stay theirs. Once
/// per daemon run (again after a failure).
pub(crate) async fn ensure_builtins(st: &AppState) {
    use std::sync::atomic::Ordering;
    if st.personas_installed.load(Ordering::SeqCst) {
        return;
    }
    let mut ok = true;
    for b in BUILTIN_PERSONAS {
        if let Err(e) = install(st, b).await {
            ok = false;
            tracing::warn!(persona = b.id, error = %e, "could not install a built-in persona");
        }
    }
    st.personas_installed.store(ok, Ordering::SeqCst);
}

async fn install(st: &AppState, b: &pdt_core::persona::BuiltinPersona) -> Result<(), pdt_store::StoreError> {
    let template = b.setup(st.registry().tool_table());
    let now = now_ms();
    let Some(p) = st.store.persona_by_builtin(b.id).await? else {
        // A persona the user already called that keeps their own; the built-in gets a free name.
        let mut name = b.name.to_string();
        let mut n = 2;
        while st.store.find_persona(&name).await?.is_some() {
            name = format!("{} {n}", b.name);
            n += 1;
        }
        let p = Persona {
            id: uuid::Uuid::now_v7().to_string(),
            name,
            role: b.role.into(),
            description: b.description.into(),
            builtin: Some(b.id.into()),
            archived: false,
            version: 1,
            setup: template.clone(),
            created_at: now,
            updated_at: now,
        };
        st.store.insert_persona(&p, PersonaAuthor::Builtin, "installed").await?;
        st.store.set_persona_template(&p.id, &template).await?;
        tracing::info!(persona = b.id, "installed a built-in persona");
        return Ok(());
    };
    // The template PDT applied last; until PDT-100 recorded it, the newest version PDT wrote.
    let base = match st.store.persona_template(&p.id).await? {
        Some(t) if t == template => return Ok(()),
        Some(t) => t,
        None => st
            .store
            .list_persona_versions(&p.id)
            .await?
            .into_iter()
            .find(|v| v.author == PersonaAuthor::Builtin)
            .map_or_else(|| template.clone(), |v| v.setup),
    };
    let (setup, kept) = pdt_core::persona::follow_template(&base, &p.setup, &template, st.registry().tool_table());
    if setup != p.setup {
        let note = match kept.as_slice() {
            [] => "updated by PDT".to_string(),
            kept => format!("updated by PDT; kept your {}", kept.join(", ")),
        };
        st.store.save_persona_version(&p.id, &setup, PersonaAuthor::Builtin, &note, now).await?;
        tracing::info!(persona = b.id, ?kept, "updated a built-in persona");
    }
    st.store.set_persona_template(&p.id, &template).await
}

/// This module's API routes; `routes::router` merges them under `/api`.
pub(crate) fn router() -> axum::Router<crate::AppState> {
    use axum::routing::get;
    axum::Router::new()
        .route("/personas", get(list).post(create))
        .route("/personas/tools", get(tools))
        .route("/personas/{id}", get(get_one).put(save).patch(self::patch).delete(remove))
        .route("/personas/{id}/versions", get(versions))
        .route("/personas/{id}/versions/{v}", get(version))
        .route("/personas/{id}/diff", get(diff))
        .route("/personas/{id}/template", get(template))
}
