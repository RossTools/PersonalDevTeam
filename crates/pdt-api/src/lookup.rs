//! Loading what a route names by its id, a missing one being 404 `not_found`.

use crate::error::{ApiError, OrNotFound};
use crate::routes::AppState;
use pdt_core::project::Project;
use pdt_core::session::Session;
use pdt_core::ticket::Ticket;

impl AppState {
    /// The project `id`.
    pub async fn project(&self, id: &str) -> Result<Project, ApiError> {
        self.store.get_project(id).await?.or_404("project")
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
