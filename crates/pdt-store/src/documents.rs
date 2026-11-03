//! Documents (PDT-314): PDT's records of the documents providers keep, their links to projects and tickets, and each
//! project's documents settings.

use crate::{Store, StoreError};
use pdt_core::document::{
    BesideTab, Chart, CommentAnchor, CommentReply, Document, DocumentComment, DocumentFilter, DocumentKind,
    DocumentProject, DocumentTicket, DocumentsBeside, ProjectDocumentSettings,
};
use pdt_core::ticket::Actor;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const SELECT: &str = "SELECT d.id, d.title, d.kind, d.provider, d.provider_ref, d.url, d.actor, d.agent, d.created_at, \
                      d.updated_at FROM documents d";

fn to_document(r: &SqliteRow) -> Result<Document, StoreError> {
    let kind: String = r.try_get("kind")?;
    let actor: String = r.try_get("actor")?;
    Ok(Document {
        id: r.try_get("id")?,
        title: r.try_get("title")?,
        kind: DocumentKind::parse(&kind).unwrap_or(DocumentKind::Doc),
        provider: r.try_get("provider")?,
        provider_ref: r.try_get("provider_ref")?,
        url: r.try_get("url")?,
        actor: Actor::parse(&actor).unwrap_or(Actor::Agent),
        agent: r.try_get::<Option<String>, _>("agent")?.and_then(|j| serde_json::from_str(&j).ok()),
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        projects: Vec::new(),
        tickets: Vec::new(),
    })
}

/// What adding a document did.
#[derive(Debug, PartialEq)]
pub enum AddDocument {
    Added,
    /// PDT has a record of that provider's file already: this one.
    Exists(String),
}

/// A link to add or remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLink {
    Project(String),
    Ticket(String),
}

impl Store {
    /// Records a document (its links are added with [`Store::link_document`]); a provider's file is recorded once.
    pub async fn add_document(&self, d: &Document) -> Result<AddDocument, StoreError> {
        let mut tx = self.write_tx().await?;
        let had: Option<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE provider = ? AND provider_ref = ?")
            .bind(&d.provider)
            .bind(&d.provider_ref)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some((id,)) = had {
            return Ok(AddDocument::Exists(id));
        }
        sqlx::query(
            "INSERT INTO documents (id, title, kind, provider, provider_ref, url, actor, agent, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&d.id)
        .bind(&d.title)
        .bind(d.kind.as_str())
        .bind(&d.provider)
        .bind(&d.provider_ref)
        .bind(&d.url)
        .bind(d.actor.as_str())
        .bind(d.agent.as_ref().map(|g| serde_json::to_string(g).expect("an agent serializes")))
        .bind(d.created_at)
        .bind(d.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(AddDocument::Added)
    }

    /// One document with its links.
    pub async fn get_document(&self, id: &str) -> Result<Option<Document>, StoreError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE d.id = ?")))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else { return Ok(None) };
        let mut d = to_document(&row)?;
        self.fill_links(std::slice::from_mut(&mut d)).await?;
        Ok(Some(d))
    }

    /// The documents a filter lets through, with their links, the most recently changed first.
    pub async fn list_documents(&self, f: &DocumentFilter) -> Result<Vec<Document>, StoreError> {
        let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(SELECT);
        qb.push(" WHERE 1 = 1");
        if let Some(p) = &f.project {
            qb.push(" AND d.id IN (SELECT document_id FROM document_projects WHERE project_id = ")
                .push_bind(p)
                .push(")");
        }
        if let Some(t) = &f.ticket {
            qb.push(" AND d.id IN (SELECT document_id FROM document_tickets WHERE ticket_id = ").push_bind(t).push(")");
        }
        if let Some(k) = f.kind {
            qb.push(" AND d.kind = ").push_bind(k.as_str());
        }
        if let Some(p) = &f.provider {
            qb.push(" AND d.provider = ").push_bind(p);
        }
        for word in f.q.as_deref().unwrap_or_default().split_whitespace() {
            let escaped = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            qb.push(" AND d.title LIKE ").push_bind(format!("%{escaped}%")).push(" ESCAPE '\\'");
        }
        qb.push(" ORDER BY d.updated_at DESC, d.id DESC");
        if let Some(n) = f.limit {
            qb.push(" LIMIT ").push_bind(i64::from(n));
        }
        let rows = qb.build().fetch_all(&self.pool).await?;
        let mut out = rows.iter().map(to_document).collect::<Result<Vec<_>, _>>()?;
        self.fill_links(&mut out).await?;
        Ok(out)
    }

    async fn fill_links(&self, docs: &mut [Document]) -> Result<(), StoreError> {
        for d in docs {
            d.projects = sqlx::query(
                "SELECT p.id, p.key, p.name FROM document_projects dp JOIN projects p ON p.id = dp.project_id \
                 WHERE dp.document_id = ? ORDER BY p.name",
            )
            .bind(&d.id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| Ok(DocumentProject { id: r.try_get("id")?, key: r.try_get("key")?, name: r.try_get("name")? }))
            .collect::<Result<_, StoreError>>()?;
            d.tickets = sqlx::query(
                "SELECT t.id, p.key || '-' || t.number AS key, t.title, t.project_id FROM document_tickets dt \
                 JOIN tickets t ON t.id = dt.ticket_id JOIN projects p ON p.id = t.project_id \
                 WHERE dt.document_id = ? ORDER BY p.key, t.number",
            )
            .bind(&d.id)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|r| {
                Ok(DocumentTicket {
                    id: r.try_get("id")?,
                    key: r.try_get("key")?,
                    title: r.try_get("title")?,
                    project_id: r.try_get("project_id")?,
                })
            })
            .collect::<Result<_, StoreError>>()?;
        }
        Ok(())
    }

    /// Links a document to a project or ticket; false when the document, project or ticket is gone. Linking twice is
    /// fine.
    pub async fn link_document(&self, id: &str, link: &DocumentLink, now: i64) -> Result<bool, StoreError> {
        let (table, column, target_table) = match link {
            DocumentLink::Project(_) => ("document_projects", "project_id", "projects"),
            DocumentLink::Ticket(_) => ("document_tickets", "ticket_id", "tickets"),
        };
        let (DocumentLink::Project(target) | DocumentLink::Ticket(target)) = link;
        let mut tx = self.write_tx().await?;
        let found: Option<(i64,)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT 1 FROM documents d, {target_table} x WHERE d.id = ? AND x.id = ?"
        )))
        .bind(id)
        .bind(target)
        .fetch_optional(&mut *tx)
        .await?;
        if found.is_none() {
            return Ok(false);
        }
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "INSERT INTO {table} (document_id, {column}) VALUES (?, ?) ON CONFLICT DO NOTHING"
        )))
        .bind(id)
        .bind(target)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE documents SET updated_at = ? WHERE id = ?").bind(now).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Removes a link; false when there was none.
    pub async fn unlink_document(&self, id: &str, link: &DocumentLink) -> Result<bool, StoreError> {
        let (sql, target) = match link {
            DocumentLink::Project(p) => ("DELETE FROM document_projects WHERE document_id = ? AND project_id = ?", p),
            DocumentLink::Ticket(t) => ("DELETE FROM document_tickets WHERE document_id = ? AND ticket_id = ?", t),
        };
        let r = sqlx::query(sql).bind(id).bind(target).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Renames a document's record; false when it is gone.
    pub async fn rename_document(&self, id: &str, title: &str, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE documents SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Marks a document changed (its content changed at the provider).
    pub async fn touch_document(&self, id: &str, now: i64) -> Result<(), StoreError> {
        sqlx::query("UPDATE documents SET updated_at = ? WHERE id = ?").bind(now).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    /// Forgets a document and its links (the provider's file is the caller's); false when it is gone.
    pub async fn delete_document(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM documents WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// A project's documents settings (none set: the host's).
    pub async fn project_document_settings(&self, project_id: &str) -> Result<ProjectDocumentSettings, StoreError> {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT provider, folder FROM project_document_settings WHERE project_id = ?")
                .bind(project_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(provider, folder)| ProjectDocumentSettings { provider, folder }).unwrap_or_default())
    }

    pub async fn put_project_document_settings(
        &self,
        project_id: &str,
        s: &ProjectDocumentSettings,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO project_document_settings (project_id, provider, folder) VALUES (?, ?, ?) \
             ON CONFLICT(project_id) DO UPDATE SET provider = excluded.provider, folder = excluded.folder",
        )
        .bind(project_id)
        .bind(&s.provider)
        .bind(&s.folder)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// A comment or reply to keep ([`Store::add_document_comment`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDocumentComment {
    pub id: String,
    pub document_id: String,
    /// The comment it replies to.
    pub parent_id: Option<String>,
    pub anchor: Option<CommentAnchor>,
    pub author: String,
    pub actor: Actor,
    pub body: String,
    pub created_at: i64,
}

impl Store {
    /// PDT's record of a provider's file, by the provider's reference.
    pub async fn document_by_ref(&self, provider: &str, reference: &str) -> Result<Option<Document>, StoreError> {
        let id: Option<(String,)> = sqlx::query_as("SELECT id FROM documents WHERE provider = ? AND provider_ref = ?")
            .bind(provider)
            .bind(reference)
            .fetch_optional(&self.pool)
            .await?;
        match id {
            Some((id,)) => self.get_document(&id).await,
            None => Ok(None),
        }
    }

    /// Keeps a comment, or a reply to one (false when the document, or the comment replied to, is gone).
    pub async fn add_document_comment(&self, c: &NewDocumentComment) -> Result<bool, StoreError> {
        if let Some(parent) = &c.parent_id {
            let top: Option<(Option<String>,)> =
                sqlx::query_as("SELECT parent_id FROM document_comments WHERE id = ? AND document_id = ?")
                    .bind(parent)
                    .bind(&c.document_id)
                    .fetch_optional(&self.pool)
                    .await?;
            // Replies hang off the top comment only.
            if !matches!(top, Some((None,))) {
                return Ok(false);
            }
        }
        let r = sqlx::query(
            "INSERT INTO document_comments (id, document_id, parent_id, anchor, author, actor, body, created_at)              SELECT ?, ?, ?, ?, ?, ?, ?, ? WHERE EXISTS (SELECT 1 FROM documents WHERE id = ?)",
        )
        .bind(&c.id)
        .bind(&c.document_id)
        .bind(&c.parent_id)
        .bind(c.anchor.as_ref().map(|a| serde_json::to_string(a).expect("an anchor serializes")))
        .bind(&c.author)
        .bind(c.actor.as_str())
        .bind(&c.body)
        .bind(c.created_at)
        .bind(&c.document_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// A document's comments, oldest first, each with its replies.
    pub async fn document_comments(&self, document_id: &str) -> Result<Vec<DocumentComment>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, parent_id, anchor, author, actor, body, resolved, created_at FROM document_comments              WHERE document_id = ? ORDER BY created_at, id",
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out: Vec<DocumentComment> = Vec::new();
        let mut replies: Vec<(String, CommentReply)> = Vec::new();
        for r in &rows {
            let id: String = r.try_get("id")?;
            let author: String = r.try_get("author")?;
            let body: String = r.try_get("body")?;
            let created_at: i64 = r.try_get("created_at")?;
            let actor = Actor::parse(&r.try_get::<String, _>("actor")?);
            match r.try_get::<Option<String>, _>("parent_id")? {
                Some(parent) => {
                    replies.push((parent, CommentReply { id, author, body, created_at: Some(created_at), actor }))
                }
                None => {
                    let anchor: Option<CommentAnchor> =
                        r.try_get::<Option<String>, _>("anchor")?.and_then(|j| serde_json::from_str(&j).ok());
                    out.push(DocumentComment {
                        id,
                        author,
                        body,
                        quote: anchor.as_ref().map(CommentAnchor::label),
                        anchor,
                        resolved: r.try_get::<i64, _>("resolved")? != 0,
                        created_at: Some(created_at),
                        replies: vec![],
                        actor,
                    });
                }
            }
        }
        for (parent, reply) in replies {
            if let Some(c) = out.iter_mut().find(|c| c.id == parent) {
                c.replies.push(reply);
            }
        }
        Ok(out)
    }

    /// Resolves a comment, or opens it again; false when it is gone.
    pub async fn resolve_document_comment(
        &self,
        document_id: &str,
        id: &str,
        resolved: bool,
    ) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE document_comments SET resolved = ? WHERE id = ? AND document_id = ? AND parent_id IS NULL",
        )
        .bind(resolved as i64)
        .bind(id)
        .bind(document_id)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    /// A spreadsheet's charts, in order, each with the sheet it is on.
    pub async fn document_charts(&self, document_id: &str) -> Result<Vec<(String, Chart)>, StoreError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT sheet, chart_id, spec FROM document_charts WHERE document_id = ? ORDER BY position, chart_id",
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(sheet, id, j)| serde_json::from_str(&j).ok().map(|spec| (sheet, Chart { id, spec })))
            .collect())
    }

    /// Adds a chart on `sheet` (last), or replaces the one with its id in place (on whatever sheet it is).
    pub async fn put_document_chart(&self, document_id: &str, sheet: &str, chart: &Chart) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO document_charts (document_id, chart_id, sheet, spec, position) \
             VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(position), 0) + 1 FROM document_charts WHERE document_id = ?)) \
             ON CONFLICT(document_id, chart_id) DO UPDATE SET spec = excluded.spec",
        )
        .bind(document_id)
        .bind(&chart.id)
        .bind(sheet)
        .bind(serde_json::to_string(&chart.spec).expect("a chart serializes"))
        .bind(document_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Moves a document's charts from one sheet name to another (a sheet renamed).
    pub async fn rename_chart_sheet(&self, document_id: &str, from: &str, to: &str) -> Result<(), StoreError> {
        sqlx::query("UPDATE document_charts SET sheet = ? WHERE document_id = ? AND sheet = ?")
            .bind(to)
            .bind(document_id)
            .bind(from)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Removes a chart; false when there was none.
    pub async fn delete_document_chart(&self, document_id: &str, chart_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM document_charts WHERE document_id = ? AND chart_id = ?")
            .bind(document_id)
            .bind(chart_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}

impl Store {
    /// The documents open beside a conversation, in tab order (PDT-317).
    pub async fn documents_beside(&self, session_id: &str) -> Result<DocumentsBeside, StoreError> {
        let rows = sqlx::query(
            "SELECT document_id, actor, opened_at, focused_at FROM documents_beside WHERE session_id = ? \
             ORDER BY position, opened_at",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        let mut tabs = Vec::with_capacity(rows.len());
        for r in rows {
            let id: String = r.try_get("document_id")?;
            let Some(document) = self.get_document(&id).await? else { continue };
            let actor: String = r.try_get("actor")?;
            tabs.push(BesideTab {
                document,
                by: Actor::parse(&actor).unwrap_or(Actor::Agent),
                opened_at: r.try_get("opened_at")?,
                focused_at: r.try_get("focused_at")?,
            });
        }
        Ok(DocumentsBeside::new(tabs))
    }

    /// Opens a document beside a conversation (a new last tab) or, when it is open there already, shows its tab.
    /// False when the session or the document is gone.
    pub async fn open_beside(
        &self,
        session_id: &str,
        document_id: &str,
        by: Actor,
        now: i64,
    ) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let there: Option<(i64,)> =
            sqlx::query_as("SELECT 1 FROM documents_beside WHERE session_id = ? AND document_id = ?")
                .bind(session_id)
                .bind(document_id)
                .fetch_optional(&mut *tx)
                .await?;
        let done = if there.is_some() {
            sqlx::query("UPDATE documents_beside SET focused_at = ? WHERE session_id = ? AND document_id = ?")
                .bind(now)
                .bind(session_id)
                .bind(document_id)
                .execute(&mut *tx)
                .await?;
            true
        } else {
            let r = sqlx::query(
                "INSERT INTO documents_beside (session_id, document_id, position, actor, opened_at, focused_at) \
                 SELECT s.id, d.id, COALESCE((SELECT MAX(position) + 1 FROM documents_beside WHERE session_id = s.id), 0), \
                        ?, ?, ? \
                 FROM sessions s, documents d WHERE s.id = ? AND d.id = ?",
            )
            .bind(by.as_str())
            .bind(now)
            .bind(now)
            .bind(session_id)
            .bind(document_id)
            .execute(&mut *tx)
            .await?;
            r.rows_affected() == 1
        };
        tx.commit().await?;
        Ok(done)
    }

    /// Closes a document's tab beside a conversation.
    pub async fn close_beside(&self, session_id: &str, document_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM documents_beside WHERE session_id = ? AND document_id = ?")
            .bind(session_id)
            .bind(document_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Puts a conversation's tabs in this order; tabs it leaves out keep their places after these.
    pub async fn order_beside(&self, session_id: &str, document_ids: &[String]) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT document_id FROM documents_beside WHERE session_id = ? ORDER BY position, opened_at",
        )
        .bind(session_id)
        .fetch_all(&mut *tx)
        .await?;
        let mut order: Vec<String> = document_ids.iter().filter(|d| rows.iter().any(|(r,)| r == *d)).cloned().collect();
        order.dedup();
        for (r,) in rows {
            if !order.contains(&r) {
                order.push(r);
            }
        }
        for (i, d) in order.iter().enumerate() {
            sqlx::query("UPDATE documents_beside SET position = ? WHERE session_id = ? AND document_id = ?")
                .bind(i as i64)
                .bind(session_id)
                .bind(d)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::document::{ChartKind, ChartSpec};
    use pdt_core::ticket::NewTicket;

    async fn store_with_ticket(dir: &std::path::Path) -> Store {
        let store = crate::worktrees::tests::store_with_project(dir).await;
        let t = NewTicket { title: "T".into(), ..Default::default() }.validate().unwrap();
        store.create_ticket("t1", "p", &t, 1).await.unwrap();
        store
    }

    fn doc(id: &str, title: &str, kind: DocumentKind, provider: &str, at: i64) -> Document {
        Document {
            id: id.into(),
            title: title.into(),
            kind,
            provider: provider.into(),
            provider_ref: format!("ref-{id}"),
            url: None,
            actor: Actor::User,
            agent: None,
            created_at: at,
            updated_at: at,
            projects: vec![],
            tickets: vec![],
        }
    }

    #[tokio::test]
    async fn documents_are_kept_linked_filtered_and_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_ticket(dir.path()).await;
        assert_eq!(
            store.add_document(&doc("d1", "Design 50%", DocumentKind::Doc, "local", 1)).await.unwrap(),
            AddDocument::Added
        );
        store.add_document(&doc("d2", "Budget", DocumentKind::Sheet, "google", 2)).await.unwrap();
        let mut again = doc("d3", "Same file", DocumentKind::Doc, "local", 3);
        again.provider_ref = "ref-d1".into();
        assert_eq!(store.add_document(&again).await.unwrap(), AddDocument::Exists("d1".into()));

        assert!(store.link_document("d1", &DocumentLink::Project("p".into()), 5).await.unwrap());
        assert!(store.link_document("d1", &DocumentLink::Project("p".into()), 5).await.unwrap());
        assert!(store.link_document("d1", &DocumentLink::Ticket("t1".into()), 5).await.unwrap());
        assert!(!store.link_document("d1", &DocumentLink::Ticket("nope".into()), 5).await.unwrap());
        assert!(!store.link_document("nope", &DocumentLink::Project("p".into()), 5).await.unwrap());

        let d1 = store.get_document("d1").await.unwrap().unwrap();
        assert_eq!(d1.projects.len(), 1);
        assert_eq!(d1.tickets.len(), 1);
        assert!(d1.tickets[0].key.ends_with("-1"), "{:?}", d1.tickets);
        assert_eq!(d1.updated_at, 5);

        let all = store.list_documents(&DocumentFilter::default()).await.unwrap();
        assert_eq!(all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["d1", "d2"]);
        let f = |f: DocumentFilter| {
            let store = store.clone();
            async move { store.list_documents(&f).await.unwrap().into_iter().map(|d| d.id).collect::<Vec<_>>() }
        };
        assert_eq!(f(DocumentFilter { ticket: Some("t1".into()), ..Default::default() }).await, ["d1"]);
        assert_eq!(f(DocumentFilter { project: Some("p".into()), ..Default::default() }).await, ["d1"]);
        assert_eq!(f(DocumentFilter { kind: Some(DocumentKind::Sheet), ..Default::default() }).await, ["d2"]);
        assert_eq!(f(DocumentFilter { provider: Some("google".into()), ..Default::default() }).await, ["d2"]);
        assert_eq!(f(DocumentFilter { q: Some("bud".into()), ..Default::default() }).await, ["d2"]);
        assert!(f(DocumentFilter { q: Some("%".into()), ..Default::default() }).await == ["d1"]);
        assert_eq!(f(DocumentFilter { limit: Some(1), ..Default::default() }).await, ["d1"]);

        assert!(store.unlink_document("d1", &DocumentLink::Ticket("t1".into())).await.unwrap());
        assert!(!store.unlink_document("d1", &DocumentLink::Ticket("t1".into())).await.unwrap());
        assert!(store.rename_document("d2", "Budget 2027", 9).await.unwrap());
        assert_eq!(store.get_document("d2").await.unwrap().unwrap().title, "Budget 2027");
        assert!(store.delete_document("d2").await.unwrap());
        assert!(store.get_document("d2").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn links_go_with_their_ticket_or_project() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_ticket(dir.path()).await;
        store.add_document(&doc("d1", "Design", DocumentKind::Doc, "local", 1)).await.unwrap();
        store.link_document("d1", &DocumentLink::Ticket("t1".into()), 2).await.unwrap();
        store.link_document("d1", &DocumentLink::Project("p".into()), 2).await.unwrap();
        store
            .put_project_document_settings(
                "p",
                &ProjectDocumentSettings { provider: Some("google".into()), folder: None },
            )
            .await
            .unwrap();
        assert_eq!(store.project_document_settings("p").await.unwrap().provider.as_deref(), Some("google"));
        store.delete_ticket("t1").await.unwrap();
        assert!(store.get_document("d1").await.unwrap().unwrap().tickets.is_empty());
        store.delete_project("p").await.unwrap();
        let d = store.get_document("d1").await.unwrap().unwrap();
        assert!(d.projects.is_empty(), "the document stays, unlinked");
        assert_eq!(store.project_document_settings("p").await.unwrap(), ProjectDocumentSettings::default());
    }

    #[tokio::test]
    async fn comments_and_charts_are_kept_with_their_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_ticket(dir.path()).await;
        store.add_document(&doc("d1", "Budget", DocumentKind::Sheet, "local", 1)).await.unwrap();
        assert_eq!(store.document_by_ref("local", "ref-d1").await.unwrap().unwrap().id, "d1");
        assert!(store.document_by_ref("google", "ref-d1").await.unwrap().is_none());

        let c = |id: &str, parent: Option<&str>, at: i64| NewDocumentComment {
            id: id.into(),
            document_id: "d1".into(),
            parent_id: parent.map(str::to_string),
            anchor: parent.is_none().then(|| CommentAnchor::Cell { sheet: "Sheet1".into(), range: "B3".into() }),
            author: "You".into(),
            actor: Actor::User,
            body: format!("body {id}"),
            created_at: at,
        };
        assert!(store.add_document_comment(&c("c1", None, 1)).await.unwrap());
        assert!(store.add_document_comment(&c("r1", Some("c1"), 2)).await.unwrap());
        assert!(!store.add_document_comment(&c("r2", Some("r1"), 3)).await.unwrap(), "no replies to replies");
        assert!(
            !store
                .add_document_comment(&NewDocumentComment { document_id: "gone".into(), ..c("x", None, 4) })
                .await
                .unwrap()
        );
        assert!(store.resolve_document_comment("d1", "c1", true).await.unwrap());
        assert!(!store.resolve_document_comment("d1", "r1", true).await.unwrap());
        let all = store.document_comments("d1").await.unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].resolved);
        assert_eq!(all[0].quote.as_deref(), Some("Sheet1!B3"));
        assert_eq!(all[0].replies.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["r1"]);

        let chart = |id: &str, title: &str| Chart {
            id: id.into(),
            spec: ChartSpec { kind: ChartKind::Bar, data: "A1:B4".into(), title: Some(title.into()), anchor: None },
        };
        store.put_document_chart("d1", "Sheet1", &chart("a", "One")).await.unwrap();
        store.put_document_chart("d1", "Sheet1", &chart("b", "Two")).await.unwrap();
        store.put_document_chart("d1", "Sheet1", &chart("a", "One again")).await.unwrap();
        store.rename_chart_sheet("d1", "Sheet1", "Data").await.unwrap();
        let charts = store.document_charts("d1").await.unwrap();
        assert_eq!(
            charts.iter().map(|(s, c)| (s.as_str(), c.spec.title.as_deref().unwrap())).collect::<Vec<_>>(),
            [("Data", "One again"), ("Data", "Two")]
        );
        assert!(store.delete_document_chart("d1", "b").await.unwrap());
        assert!(!store.delete_document_chart("d1", "b").await.unwrap());

        store.delete_document("d1").await.unwrap();
        assert!(store.document_comments("d1").await.unwrap().is_empty());
        assert!(store.document_charts("d1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn documents_open_beside_a_conversation_in_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_ticket(dir.path()).await;
        store.insert_session(&crate::sessions::tests::session("s1", 1)).await.unwrap();
        for (id, at) in [("d1", 1), ("d2", 2), ("d3", 3)] {
            store.add_document(&doc(id, id, DocumentKind::Doc, "local", at)).await.unwrap();
        }
        assert!(store.documents_beside("s1").await.unwrap().tabs.is_empty());
        assert!(store.open_beside("s1", "d1", Actor::User, 10).await.unwrap());
        assert!(store.open_beside("s1", "d2", Actor::Agent, 11).await.unwrap());
        assert!(store.open_beside("s1", "d3", Actor::User, 12).await.unwrap());
        assert!(!store.open_beside("s1", "nope", Actor::User, 12).await.unwrap());
        assert!(!store.open_beside("gone", "d1", Actor::User, 12).await.unwrap());
        let ids = |b: &DocumentsBeside| b.tabs.iter().map(|t| t.document.id.clone()).collect::<Vec<_>>();
        let b = store.documents_beside("s1").await.unwrap();
        assert_eq!(ids(&b), ["d1", "d2", "d3"]);
        assert_eq!(b.active.as_deref(), Some("d3"));
        assert_eq!(b.tabs[1].by, Actor::Agent);

        // Opening one again shows it, in its place.
        store.open_beside("s1", "d1", Actor::Agent, 13).await.unwrap();
        let b = store.documents_beside("s1").await.unwrap();
        assert_eq!((ids(&b), b.active.as_deref()), (vec!["d1".into(), "d2".into(), "d3".into()], Some("d1")));
        assert_eq!(b.tabs[0].by, Actor::User, "who opened it first stays");

        store.order_beside("s1", &["d3".into(), "d1".into(), "x".into()]).await.unwrap();
        assert_eq!(ids(&store.documents_beside("s1").await.unwrap()), ["d3", "d1", "d2"]);
        assert!(store.close_beside("s1", "d1").await.unwrap());
        assert!(!store.close_beside("s1", "d1").await.unwrap());
        store.delete_document("d3").await.unwrap();
        let b = store.documents_beside("s1").await.unwrap();
        assert_eq!((ids(&b), b.active.as_deref()), (vec!["d2".into()], Some("d2")));
    }
}
