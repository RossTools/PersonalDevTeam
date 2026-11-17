//! Which document changes an agent makes without asking the user (PDT-618): the project's
//! [`AgentDocuments`] level against what a call would touch. Pure rules; pdt-api resolves the call's documents,
//! projects and tickets to ids first.

use crate::policy::AgentDocuments;

/// The agent making the call: the project its session is in, and the ticket it works on.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    pub project: &'a str,
    pub ticket: Option<&'a str>,
}

/// What a document tool's call touches, by id.
#[derive(Debug, Clone, Copy)]
pub enum DocumentCall<'a> {
    /// A new document linked to this project and ticket (`document_create`, after it fills in the session's own
    /// ticket when it is given neither).
    Create { project: Option<&'a str>, ticket: Option<&'a str> },
    /// A change to a document linked to these projects and tickets (`doc_edit`, `sheet_edit`).
    Change { projects: &'a [String], tickets: &'a [String] },
    /// Linking such a document to a project or ticket, or unlinking it (`document_link`).
    Link { projects: &'a [String], tickets: &'a [String], project: Option<&'a str>, ticket: Option<&'a str> },
}

/// Whether `level` lets `caller` make `call` without asking.
pub fn allows(level: AgentDocuments, caller: Caller<'_>, call: DocumentCall<'_>) -> bool {
    match call {
        DocumentCall::Create { project, ticket } => may_link_to(level, caller, project, ticket),
        DocumentCall::Change { projects, tickets } => may_change(level, caller, projects, tickets),
        DocumentCall::Link { projects, tickets, project, ticket } => {
            may_change(level, caller, projects, tickets) && may_link_to(level, caller, project, ticket)
        }
    }
}

/// Whether a document may be linked to `project` and `ticket` (a new one, or a link added or removed).
fn may_link_to(level: AgentDocuments, caller: Caller<'_>, project: Option<&str>, ticket: Option<&str>) -> bool {
    let own_ticket = |t: &str| caller.ticket == Some(t);
    let own_project = |p: &str| p == caller.project;
    match level {
        AgentDocuments::Off => false,
        // Its own ticket (and that ticket's project, which a ticket's document is linked to as well).
        AgentDocuments::OwnTicket => ticket.is_some_and(own_ticket) && project.is_none_or(own_project),
        AgentDocuments::Project => {
            (ticket.is_some() || project.is_some()) && ticket.is_none_or(own_ticket) && project.is_none_or(own_project)
        }
    }
}

/// Whether a document linked to `projects` and `tickets` may be changed.
fn may_change(level: AgentDocuments, caller: Caller<'_>, projects: &[String], tickets: &[String]) -> bool {
    match level {
        AgentDocuments::Off => false,
        AgentDocuments::OwnTicket => caller.ticket.is_some_and(|own| tickets.iter().any(|t| t == own)),
        AgentDocuments::Project => {
            projects.iter().any(|p| p == caller.project)
                || caller.ticket.is_some_and(|own| tickets.iter().any(|t| t == own))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentDocuments::*;

    const ME: Caller<'static> = Caller { project: "P", ticket: Some("T1") };
    const NO_TICKET: Caller<'static> = Caller { project: "P", ticket: None };

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn create(level: AgentDocuments, caller: Caller<'_>, project: Option<&str>, ticket: Option<&str>) -> bool {
        allows(level, caller, DocumentCall::Create { project, ticket })
    }

    fn change(level: AgentDocuments, caller: Caller<'_>, projects: &[&str], tickets: &[&str]) -> bool {
        let (p, t) = (strings(projects), strings(tickets));
        allows(level, caller, DocumentCall::Change { projects: &p, tickets: &t })
    }

    #[test]
    fn off_always_asks() {
        assert!(!create(Off, ME, None, Some("T1")));
        assert!(!change(Off, ME, &["P"], &["T1"]));
        let (p, t) = (strings(&["P"]), strings(&["T1"]));
        assert!(!allows(Off, ME, DocumentCall::Link { projects: &p, tickets: &t, project: None, ticket: Some("T1") }));
    }

    #[test]
    fn own_ticket_creates_and_changes_only_its_tickets_documents() {
        assert!(create(OwnTicket, ME, None, Some("T1")));
        assert!(create(OwnTicket, ME, Some("P"), Some("T1")), "its ticket and that ticket's project");
        assert!(!create(OwnTicket, ME, None, Some("T2")), "another ticket");
        assert!(!create(OwnTicket, ME, Some("P"), None), "the project alone");
        assert!(!create(OwnTicket, ME, Some("Q"), Some("T1")), "another project");
        assert!(!create(OwnTicket, NO_TICKET, Some("P"), None), "a session with no ticket");
        assert!(change(OwnTicket, ME, &["P"], &["T1"]));
        assert!(change(OwnTicket, ME, &["P"], &["T2", "T1"]));
        assert!(!change(OwnTicket, ME, &["P"], &["T2"]), "another ticket's");
        assert!(!change(OwnTicket, ME, &["P"], &[]), "a project-only document");
        assert!(!change(OwnTicket, NO_TICKET, &["P"], &[]));
    }

    #[test]
    fn project_creates_in_the_project_or_its_ticket_and_changes_the_projects_documents() {
        assert!(create(Project, ME, Some("P"), None));
        assert!(create(Project, ME, None, Some("T1")));
        assert!(create(Project, NO_TICKET, Some("P"), None));
        assert!(!create(Project, ME, None, Some("T2")), "another ticket");
        assert!(!create(Project, ME, Some("Q"), None), "another project");
        assert!(!create(Project, NO_TICKET, None, None), "linked to nothing");
        assert!(change(Project, ME, &["P"], &[]));
        assert!(change(Project, ME, &["Q", "P"], &["T9"]));
        assert!(change(Project, ME, &[], &["T1"]));
        assert!(!change(Project, ME, &["Q"], &["T9"]), "another project's");
    }

    #[test]
    fn links_need_both_the_document_and_the_target_in_reach() {
        let link = |level, projects: &[&str], tickets: &[&str], project, ticket| {
            let (p, t) = (strings(projects), strings(tickets));
            allows(level, ME, DocumentCall::Link { projects: &p, tickets: &t, project, ticket })
        };
        assert!(link(OwnTicket, &["P"], &["T1"], None, Some("T1")));
        assert!(!link(OwnTicket, &["P"], &["T1"], None, Some("T2")), "to another ticket");
        assert!(!link(OwnTicket, &["P"], &[], None, Some("T1")), "a project document onto its ticket");
        assert!(link(Project, &["P"], &[], None, Some("T1")));
        assert!(link(Project, &["P"], &[], Some("P"), None));
        assert!(!link(Project, &["P"], &[], Some("Q"), None), "to another project");
        assert!(!link(Project, &["Q"], &[], None, Some("T1")), "another project's document");
    }

    #[test]
    fn levels_read_and_write_as_text() {
        for l in AgentDocuments::ALL {
            assert_eq!(AgentDocuments::parse(l.as_str()), Some(l));
        }
        assert_eq!(serde_json::to_value(OwnTicket).unwrap(), "own_ticket");
        assert_eq!(AgentDocuments::default(), Off);
    }
}
