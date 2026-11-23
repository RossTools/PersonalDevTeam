//! Loading what a route names by its id, a missing one being 404 `not_found`.

use crate::error::{ApiError, OrNotFound};
use crate::routes::AppState;
use pdt_core::project::Project;
use pdt_core::session::Session;
use pdt_core::ticket::Ticket;

impl AppState {
    /// The project `id`: one of this instance's own. A remote project (another instance's, PDT-631) is refused, so
    /// no route of this instance's own works on it: only `remote_projects` does.
    pub async fn project(&self, id: &str) -> Result<Project, ApiError> {
        if let Some(p) = self.store.get_project(id).await? {
            return Ok(p);
        }
        match self.store.get_remote_project(id).await? {
            Some(p) => Err(crate::error::ApiError::conflict(
                crate::error::codes::REMOTE_PROJECT,
                format!("{} is a remote project: its tickets and settings are on the instance that owns it", p.name),
            )),
            None => Err(crate::error::ApiError::not_found("project")),
        }
    }

    /// The ticket `id`, without its overlaps ([`crate::overlap::load_ticket`] adds them).
    pub async fn ticket(&self, id: &str) -> Result<Ticket, ApiError> {
        self.store.get_ticket(id).await?.or_404("ticket")
    }

    /// The session `id`.
    pub async fn session(&self, id: &str) -> Result<Session, ApiError> {
        self.store.get_session(id).await?.or_404("session")
    }
}
