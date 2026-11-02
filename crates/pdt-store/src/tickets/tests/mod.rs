use super::*;
use crate::worktrees::tests::store_with_project;
use pdt_core::project::Project;
use pdt_core::ticket::TicketQuery;
use pdt_core::workspace::TabItem;

mod attempts;
mod create;
mod edits;
mod events;
mod links;
mod overlap;
mod parts;
mod query;

fn new(title: &str) -> ValidNewTicket {
    ValidNewTicket {
        title: title.into(),
        description: String::new(),
        stage: TicketStage::Draft,
        priority: Priority::None,
        labels: vec![],
        areas: vec![],
        before: None,
        after: None,
        parent: None,
        blocked_by_previous: false,
        source: TicketSource::User,
    }
}

fn part(title: &str, parent: &str) -> ValidNewTicket {
    ValidNewTicket { parent: Some(parent.into()), ..new(title) }
}

async fn add(store: &Store, id: &str, t: ValidNewTicket) -> Ticket {
    store.create_ticket(id, "p", &t, 1).await.unwrap()
}

fn filter(q: TicketQuery) -> TicketFilter {
    q.validate().unwrap()
}

async fn ids(store: &Store, q: TicketQuery) -> Vec<String> {
    store.query_tickets(&filter(q)).await.unwrap().items.into_iter().map(|t| t.id).collect()
}

fn in_project() -> TicketQuery {
    TicketQuery { project: Some("p".into()), ..Default::default() }
}

async fn events(store: &Store, id: &str) -> Vec<EventKind> {
    store.list_events(id, None, 100, false).await.unwrap().unwrap().items.into_iter().map(|e| e.kind).collect()
}
