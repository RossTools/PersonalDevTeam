//! Workspaces and their tabs (workspaces spec §2, §5). Positions are kept as 0..n within each list and rewritten in
//! one transaction whenever the order changes.

use crate::{Store, StoreError};
use pdt_core::ticket::{AttemptState, TicketStage};
use pdt_core::workspace::{Tab, TabItem, Workspace, WorkspaceTicket, place_at};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, Sqlite, SqliteConnection, Transaction};
use std::collections::HashMap;

/// Why a workspace was not deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteWorkspace {
    Deleted,
    NotFound,
    /// A project's own workspace goes only with its project.
    ProjectWorkspace,
}

fn to_tab(r: &SqliteRow) -> Result<Tab, StoreError> {
    Ok(Tab {
        id: r.try_get("id")?,
        workspace_id: r.try_get("workspace_id")?,
        session_id: r.try_get("session_id")?,
        run_id: r.try_get("run_id")?,
        ticket_id: r.try_get("ticket_id")?,
        worktree_id: r.try_get("worktree_id")?,
        position: u32::try_from(r.try_get::<i64, _>("position")?).unwrap_or(u32::MAX),
    })
}

const TAB_COLUMNS: &str = "id, workspace_id, session_id, run_id, ticket_id, worktree_id, position";

async fn tab_ids(conn: &mut SqliteConnection, workspace_id: &str) -> Result<Vec<String>, StoreError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM workspace_tabs WHERE workspace_id = ? ORDER BY position, created_at, id")
            .bind(workspace_id)
            .fetch_all(&mut *conn)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Puts `tab_id` into `workspace_id` at `index` and renumbers that workspace's tabs (and the one it left).
async fn place_tab(
    tx: &mut Transaction<'_, Sqlite>,
    tab_id: &str,
    from: Option<&str>,
    workspace_id: &str,
    index: Option<u32>,
) -> Result<(), StoreError> {
    let ids = tab_ids(tx, workspace_id).await?;
    let order = place_at(&ids, tab_id, index.map(|i| i as usize));
    for (pos, id) in order.iter().enumerate() {
        sqlx::query("UPDATE workspace_tabs SET workspace_id = ?, position = ? WHERE id = ?")
            .bind(workspace_id)
            .bind(pos as i64)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    if let Some(from) = from.filter(|f| *f != workspace_id) {
        renumber_tabs(tx, from).await?;
    }
    Ok(())
}

pub(crate) async fn renumber_tabs(conn: &mut SqliteConnection, workspace_id: &str) -> Result<(), StoreError> {
    for (pos, id) in tab_ids(conn, workspace_id).await?.iter().enumerate() {
        sqlx::query("UPDATE workspace_tabs SET position = ? WHERE id = ?")
            .bind(pos as i64)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

async fn renumber_workspaces(tx: &mut Transaction<'_, Sqlite>, moving: Option<(&str, u32)>) -> Result<(), StoreError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM workspaces ORDER BY position, created_at, id").fetch_all(&mut **tx).await?;
    let mut ids: Vec<String> = rows.into_iter().map(|(id,)| id).collect();
    if let Some((id, index)) = moving {
        ids = place_at(&ids, id, Some(index as usize));
    }
    for (pos, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE workspaces SET position = ? WHERE id = ?")
            .bind(pos as i64)
            .bind(id)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Adds a project's own workspace at the end, inside the transaction that adds the project.
pub(crate) async fn insert_project_workspace(
    tx: &mut Transaction<'_, Sqlite>,
    project_id: &str,
    name: &str,
    created_at: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO workspaces (id, name, project_id, position, created_at) \
         VALUES (lower(hex(randomblob(16))), ?, ?, (SELECT COALESCE(MAX(position) + 1, 0) FROM workspaces), ?)",
    )
    .bind(name)
    .bind(project_id)
    .bind(created_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

impl Store {
    /// Every workspace with its tabs, in order.
    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, name, project_id, position, created_at FROM workspaces ORDER BY position, created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        let sql = format!("SELECT {TAB_COLUMNS} FROM workspace_tabs ORDER BY position, created_at, id");
        let tabs = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        let tabs: Vec<Tab> = tabs.iter().map(to_tab).collect::<Result<_, _>>()?;
        let tickets = self.workspace_tickets().await?;
        rows.iter()
            .map(|r| {
                let id: String = r.try_get("id")?;
                Ok(Workspace {
                    tabs: tabs.iter().filter(|t| t.workspace_id == id).cloned().collect(),
                    name: r.try_get("name")?,
                    project_id: r.try_get("project_id")?,
                    position: u32::try_from(r.try_get::<i64, _>("position")?).unwrap_or(u32::MAX),
                    created_at: r.try_get("created_at")?,
                    ticket: tickets.get(&id).cloned(),
                    id,
                })
            })
            .collect()
    }

    /// The ticket of each workspace a Start made (the newest attempt naming it), by workspace id.
    async fn workspace_tickets(&self) -> Result<HashMap<String, WorkspaceTicket>, StoreError> {
        let rows = sqlx::query(
            "SELECT a.workspace_id, a.id AS attempt_id, a.state, t.id, t.project_id, t.number, t.title, t.stage, \
             COALESCE(p.key, '') AS pkey FROM attempts a JOIN tickets t ON t.id = a.ticket_id \
             JOIN projects p ON p.id = t.project_id WHERE a.workspace_id IS NOT NULL ORDER BY a.created_at, a.id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = HashMap::new();
        for r in &rows {
            let state: String = r.try_get("state")?;
            let stage: String = r.try_get("stage")?;
            let number: i64 = r.try_get("number")?;
            let pkey: String = r.try_get("pkey")?;
            let t = WorkspaceTicket {
                id: r.try_get("id")?,
                project_id: r.try_get("project_id")?,
                key: format!("{pkey}-{number}"),
                title: r.try_get("title")?,
                stage: TicketStage::parse(&stage)
                    .ok_or_else(|| StoreError::Corrupt(format!("ticket stage {stage}")))?,
                attempt_id: r.try_get("attempt_id")?,
                attempt_state: AttemptState::parse(&state)
                    .ok_or_else(|| StoreError::Corrupt(format!("attempt state {state}")))?,
                pr: None,
            };
            out.insert(r.try_get::<String, _>("workspace_id")?, t);
        }
        Ok(out)
    }

    pub async fn get_workspace(&self, id: &str) -> Result<Option<Workspace>, StoreError> {
        Ok(self.list_workspaces().await?.into_iter().find(|w| w.id == id))
    }

    /// The id of a project's own workspace.
    pub async fn project_workspace_id(&self, project_id: &str) -> Result<Option<String>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT id FROM workspaces WHERE project_id = ?")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Adds a free-form workspace at the end. `w.position` and `w.tabs` are ignored.
    pub async fn insert_workspace(&self, w: &Workspace) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO workspaces (id, name, project_id, position, created_at) \
             VALUES (?, ?, NULL, (SELECT COALESCE(MAX(position) + 1, 0) FROM workspaces), ?)",
        )
        .bind(&w.id)
        .bind(&w.name)
        .bind(w.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rename_workspace(&self, id: &str, name: &str) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE workspaces SET name = ? WHERE id = ?").bind(name).bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn move_workspace(&self, id: &str, index: u32) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM workspaces WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
        if exists.is_none() {
            return Ok(false);
        }
        renumber_workspaces(&mut tx, Some((id, index))).await?;
        tx.commit().await?;
        Ok(true)
    }

    /// Deletes a free-form workspace, moving its tabs to the end of their projects' workspaces, or closing them with
    /// `close_tabs` (their sessions, runs and tickets stay).
    pub async fn delete_workspace(&self, id: &str, close_tabs: bool) -> Result<DeleteWorkspace, StoreError> {
        let mut tx = self.write_tx().await?;
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT project_id FROM workspaces WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
        match row {
            None => return Ok(DeleteWorkspace::NotFound),
            Some((Some(_),)) => return Ok(DeleteWorkspace::ProjectWorkspace),
            Some((None,)) => {}
        }
        if close_tabs {
            sqlx::query("DELETE FROM workspace_tabs WHERE workspace_id = ?").bind(id).execute(&mut *tx).await?;
        }
        let homes: Vec<(String, String)> = sqlx::query_as(
            "SELECT t.id, w.id FROM workspace_tabs t \
             LEFT JOIN sessions s ON s.id = t.session_id \
             LEFT JOIN runs r ON r.id = t.run_id \
             LEFT JOIN tickets k ON k.id = t.ticket_id \
             LEFT JOIN worktrees g ON g.id = t.worktree_id \
             JOIN workspaces w ON w.project_id = COALESCE(s.project_id, r.project_id, k.project_id, g.project_id) \
             WHERE t.workspace_id = ? ORDER BY t.position, t.created_at, t.id",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        for (tab, home) in &homes {
            place_tab(&mut tx, tab, None, home, None).await?;
        }
        sqlx::query("DELETE FROM workspaces WHERE id = ?").bind(id).execute(&mut *tx).await?;
        renumber_workspaces(&mut tx, None).await?;
        tx.commit().await?;
        Ok(DeleteWorkspace::Deleted)
    }

    pub async fn get_tab(&self, id: &str) -> Result<Option<Tab>, StoreError> {
        let sql = format!("SELECT {TAB_COLUMNS} FROM workspace_tabs WHERE id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_tab).transpose()
    }

    /// The tab showing this session, run or ticket, if it has one.
    pub async fn tab_for(&self, item: &TabItem) -> Result<Option<Tab>, StoreError> {
        let (col, id) = match item {
            TabItem::Session(id) => ("session_id", id),
            TabItem::Run(id) => ("run_id", id),
            TabItem::Ticket(id) => ("ticket_id", id),
            TabItem::Changes(id) => ("worktree_id", id),
        };
        let sql = format!("SELECT {TAB_COLUMNS} FROM workspace_tabs WHERE {col} = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_tab).transpose()
    }

    /// Opens a tab for `item` in `workspace_id` at `index` (the end when None). An item that already has a tab moves
    /// it there instead, keeping its id; `new_id` is used only for a new tab. None if the workspace doesn't exist.
    /// The item must exist (callers check).
    pub async fn open_tab(
        &self,
        new_id: &str,
        workspace_id: &str,
        item: &TabItem,
        index: Option<u32>,
        now: i64,
    ) -> Result<Option<Tab>, StoreError> {
        let mut tx = self.write_tx().await?;
        let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM workspaces WHERE id = ?")
            .bind(workspace_id)
            .fetch_optional(&mut *tx)
            .await?;
        if exists.is_none() {
            return Ok(None);
        }
        let (session_id, run_id, ticket_id, worktree_id) = match item {
            TabItem::Session(id) => (Some(id.as_str()), None, None, None),
            TabItem::Run(id) => (None, Some(id.as_str()), None, None),
            TabItem::Ticket(id) => (None, None, Some(id.as_str()), None),
            TabItem::Changes(id) => (None, None, None, Some(id.as_str())),
        };
        let current: Option<(String, String)> = sqlx::query_as(
            "SELECT id, workspace_id FROM workspace_tabs WHERE session_id IS ? AND run_id IS ? AND ticket_id IS ? \
             AND worktree_id IS ? \
             AND (session_id IS NOT NULL OR run_id IS NOT NULL OR ticket_id IS NOT NULL OR worktree_id IS NOT NULL)",
        )
        .bind(session_id)
        .bind(run_id)
        .bind(ticket_id)
        .bind(worktree_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (tab_id, from) = match current {
            Some((id, from)) => (id, Some(from)),
            None => {
                sqlx::query(
                    "INSERT INTO workspace_tabs \
                     (id, workspace_id, session_id, run_id, ticket_id, worktree_id, position, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?, (SELECT COUNT(*) FROM workspace_tabs WHERE workspace_id = ?), ?)",
                )
                .bind(new_id)
                .bind(workspace_id)
                .bind(session_id)
                .bind(run_id)
                .bind(ticket_id)
                .bind(worktree_id)
                .bind(workspace_id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                (new_id.to_string(), None)
            }
        };
        place_tab(&mut tx, &tab_id, from.as_deref(), workspace_id, index).await?;
        tx.commit().await?;
        self.get_tab(&tab_id).await
    }

    /// Moves a tab within its workspace or to another one. None if the tab or the target workspace doesn't exist.
    pub async fn move_tab(
        &self,
        id: &str,
        workspace_id: Option<&str>,
        index: Option<u32>,
    ) -> Result<Option<Tab>, StoreError> {
        let Some(tab) = self.get_tab(id).await? else { return Ok(None) };
        let target = workspace_id.unwrap_or(&tab.workspace_id).to_string();
        let mut tx = self.write_tx().await?;
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM workspaces WHERE id = ?").bind(&target).fetch_optional(&mut *tx).await?;
        if exists.is_none() {
            return Ok(None);
        }
        // Moving to another workspace without a position goes to its end; within one, staying put is the default.
        let index = index.or(if target == tab.workspace_id { Some(tab.position) } else { None });
        place_tab(&mut tx, id, Some(&tab.workspace_id), &target, index).await?;
        tx.commit().await?;
        self.get_tab(id).await
    }

    /// Closes the gaps a deleted session or run leaves in a workspace's tab positions.
    pub async fn renumber_workspace_tabs(&self, workspace_id: &str) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        renumber_tabs(&mut tx, workspace_id).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn close_tab(&self, id: &str) -> Result<bool, StoreError> {
        let Some(tab) = self.get_tab(id).await? else { return Ok(false) };
        let mut tx = self.write_tx().await?;
        sqlx::query("DELETE FROM workspace_tabs WHERE id = ?").bind(id).execute(&mut *tx).await?;
        renumber_tabs(&mut tx, &tab.workspace_id).await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;
    use pdt_core::project::Project;
    use pdt_core::session::{Session, SessionKind, SessionStatus};

    fn session(id: &str, project: &str) -> Session {
        Session {
            id: id.into(),
            project_id: project.into(),
            worktree_id: None,
            kind: SessionKind::Shell,
            agent: None,
            bypass_permissions: false,
            title: "Shell".into(),
            cwd: "/r".into(),
            status: SessionStatus::Running,
            exit_code: None,
            created_at: 1,
            ended_at: None,
            external_session_id: None,
            fresh_started_at: None,
            activity: None,
            attention_at: None,
            last_viewed_at: None,
            unseen: false,
            attempt_id: None,
            ticket: None,
            mode: pdt_core::conversation::SessionMode::Terminal,
            model: None,
            permission_mode: None,
            persona: None,
        }
    }

    fn free(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.into(),
            name: name.into(),
            project_id: None,
            position: 0,
            created_at: 5,
            tabs: vec![],
            ticket: None,
        }
    }

    fn s(id: &str) -> TabItem {
        TabItem::Session(id.into())
    }

    fn tab_items(w: &Workspace) -> Vec<String> {
        w.tabs.iter().map(|t| t.session_id.clone().or(t.run_id.clone()).or(t.ticket_id.clone()).unwrap()).collect()
    }

    async fn ws(store: &Store, id: &str) -> Workspace {
        store.get_workspace(id).await.unwrap().unwrap()
    }

    #[tokio::test]
    async fn a_project_gets_its_workspace_and_loses_it_with_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let list = store.list_workspaces().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].project_id.as_deref(), list[0].position), ("P", Some("p"), 0));
        assert_eq!(store.project_workspace_id("p").await.unwrap(), Some(list[0].id.clone()));

        let q = Project {
            id: "q".into(),
            name: "Q".into(),
            repo_path: "/q".into(),
            base_branch: "main".into(),
            created_at: 2,
            key: String::new(),
            forge: None,
            kind: Default::default(),
        };
        store.insert_project(&q).await.unwrap();
        let list = store.list_workspaces().await.unwrap();
        assert_eq!(list.iter().map(|w| (w.name.as_str(), w.position)).collect::<Vec<_>>(), [("P", 0), ("Q", 1)]);

        sqlx::query("DELETE FROM projects WHERE id = 'q'").execute(&store.pool).await.unwrap();
        assert_eq!(store.list_workspaces().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn tabs_open_in_order_and_an_item_has_one_tab() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let home = store.project_workspace_id("p").await.unwrap().unwrap();
        for id in ["a", "b", "c"] {
            store.insert_session(&session(id, "p")).await.unwrap();
        }
        store.open_tab("t1", &home, &s("a"), None, 1).await.unwrap().unwrap();
        store.open_tab("t2", &home, &s("b"), None, 1).await.unwrap().unwrap();
        store.open_tab("t3", &home, &s("c"), Some(1), 1).await.unwrap().unwrap();
        assert_eq!(tab_items(&ws(&store, &home).await), ["a", "c", "b"]);
        assert_eq!(ws(&store, &home).await.tabs.iter().map(|t| t.position).collect::<Vec<_>>(), [0, 1, 2]);

        // Opening it elsewhere moves the existing tab, keeping its id.
        store.insert_workspace(&free("f", "Release")).await.unwrap();
        let moved = store.open_tab("ignored", "f", &s("c"), None, 2).await.unwrap().unwrap();
        assert_eq!((moved.id.as_str(), moved.workspace_id.as_str(), moved.position), ("t3", "f", 0));
        assert_eq!(tab_items(&ws(&store, &home).await), ["a", "b"]);
        assert_eq!(ws(&store, &home).await.tabs[1].position, 1);
        assert_eq!(store.tab_for(&s("c")).await.unwrap().unwrap().workspace_id, "f");
        assert_eq!(store.open_tab("x", "nope", &s("a"), None, 1).await.unwrap(), None);
    }

    #[tokio::test]
    async fn tabs_move_close_and_go_with_their_session() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let home = store.project_workspace_id("p").await.unwrap().unwrap();
        for id in ["a", "b", "c"] {
            store.insert_session(&session(id, "p")).await.unwrap();
            store.open_tab(&format!("t{id}"), &home, &s(id), None, 1).await.unwrap();
        }
        store.move_tab("ta", None, Some(2)).await.unwrap().unwrap();
        assert_eq!(tab_items(&ws(&store, &home).await), ["b", "c", "a"]);
        store.insert_workspace(&free("f", "F")).await.unwrap();
        store.move_tab("tb", Some("f"), None).await.unwrap().unwrap();
        assert_eq!(tab_items(&ws(&store, "f").await), ["b"]);
        assert_eq!(tab_items(&ws(&store, &home).await), ["c", "a"]);
        assert_eq!(store.move_tab("tb", Some("nope"), None).await.unwrap(), None);
        assert_eq!(store.move_tab("nope", None, None).await.unwrap(), None);

        assert!(store.close_tab("tc").await.unwrap());
        assert!(!store.close_tab("tc").await.unwrap());
        assert_eq!(ws(&store, &home).await.tabs[0].position, 0);

        store.delete_session("a").await.unwrap();
        assert!(ws(&store, &home).await.tabs.is_empty());
    }

    #[tokio::test]
    async fn workspaces_rename_move_and_delete_sending_tabs_home() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let home = store.project_workspace_id("p").await.unwrap().unwrap();
        store.insert_workspace(&free("f", "F")).await.unwrap();
        store.insert_workspace(&free("g", "G")).await.unwrap();
        assert!(store.rename_workspace("f", "Release").await.unwrap());
        assert!(store.move_workspace("g", 0).await.unwrap());
        assert!(!store.move_workspace("nope", 0).await.unwrap());
        let names: Vec<String> = store.list_workspaces().await.unwrap().into_iter().map(|w| w.name).collect();
        assert_eq!(names, ["G", "P", "Release"]);

        for id in ["a", "b"] {
            store.insert_session(&session(id, "p")).await.unwrap();
        }
        store.open_tab("ta", &home, &s("a"), None, 1).await.unwrap();
        store.open_tab("tb", "f", &s("b"), None, 1).await.unwrap();
        assert_eq!(store.delete_workspace(&home, false).await.unwrap(), DeleteWorkspace::ProjectWorkspace);
        assert_eq!(store.delete_workspace("nope", false).await.unwrap(), DeleteWorkspace::NotFound);
        assert_eq!(store.delete_workspace("f", false).await.unwrap(), DeleteWorkspace::Deleted);
        assert_eq!(tab_items(&ws(&store, &home).await), ["a", "b"]);
        let list = store.list_workspaces().await.unwrap();
        assert_eq!(list.iter().map(|w| (w.name.as_str(), w.position)).collect::<Vec<_>>(), [("G", 0), ("P", 1)]);

        // Closing the tabs instead: they are gone, their sessions stay.
        store.open_tab("tb2", "g", &s("b"), None, 2).await.unwrap();
        assert_eq!(store.delete_workspace("g", true).await.unwrap(), DeleteWorkspace::Deleted);
        assert_eq!(tab_items(&ws(&store, &home).await), ["a"]);
        assert!(store.get_session("b").await.unwrap().is_some());
        assert_eq!(store.list_workspaces().await.unwrap().len(), 1);
    }

    /// Applies migrations up to 0007, adds rows, then opens with 0008 and checks the seeded workspaces and tabs.
    #[tokio::test]
    async fn the_migration_gives_existing_projects_a_workspace_with_their_tabs() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("m");
        std::fs::create_dir_all(&old).unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.as_str() < "0008" {
                std::fs::copy(e.path(), old.join(&name)).unwrap();
            }
        }
        let db = tmp.path().join("pdt.db");
        let backups = tmp.path().join("backups");
        let migrator = sqlx::migrate::Migrator::new(old.as_path()).await.unwrap();
        let store = Store::open_with(&db, &migrator, &backups).await.unwrap();
        let exec = |sql: &'static str| sqlx::query(sql).execute(&store.pool);
        exec("INSERT INTO projects VALUES ('p2', 'Second', '/b', 'main', 20), ('p1', 'First', '/a', 'main', 10)")
            .await
            .unwrap();
        exec("INSERT INTO worktrees VALUES ('w1', 'p1', 'feat', '/a/.worktrees/feat', 'abc', 11)").await.unwrap();
        exec(
            "INSERT INTO sessions (id, project_id, worktree_id, kind, agent, title, cwd, status, created_at) VALUES \
             ('s-wt', 'p1', 'w1', 'shell', NULL, 'Shell', '/a', 'running', 5), \
             ('s-main2', 'p1', NULL, 'shell', NULL, 'Shell', '/a', 'finished', 9), \
             ('s-main1', 'p1', NULL, 'shell', NULL, 'Shell', '/a', 'running', 7)",
        )
        .await
        .unwrap();
        exec(
            "INSERT INTO runs (id, project_id, worktree_id, branch, name, kind, state, cwd, created_at, updated_at) \
             VALUES ('r-live', 'p1', 'w1', 'feat', 'pdt', 'run', 'ready', '/a', 6, 6), \
                    ('r-old', 'p1', NULL, 'main', 'pdt', 'run', 'stopped', '/a', 1, 1)",
        )
        .await
        .unwrap();
        store.close().await;

        let store = Store::open_with(&db, &crate::migrate::MIGRATOR, &backups).await.unwrap();
        let list = store.list_workspaces().await.unwrap();
        assert_eq!(
            list.iter().map(|w| (w.name.as_str(), w.position)).collect::<Vec<_>>(),
            [("First", 0), ("Second", 1)]
        );
        assert_eq!(tab_items(&list[0]), ["s-main1", "s-main2", "s-wt", "r-live"]);
        assert_eq!(list[0].tabs.iter().map(|t| t.position).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert!(list[1].tabs.is_empty());
    }

    /// Applies migrations up to 0009, adds tabs, then opens with 0010 (which rebuilds workspace_tabs) and checks they
    /// survive with their constraints.
    #[tokio::test]
    async fn the_tickets_migration_keeps_existing_tabs() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("m");
        std::fs::create_dir_all(&old).unwrap();
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        for e in std::fs::read_dir(&src).unwrap() {
            let e = e.unwrap();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.as_str() < "0010" {
                std::fs::copy(e.path(), old.join(&name)).unwrap();
            }
        }
        let db = tmp.path().join("pdt.db");
        let backups = tmp.path().join("backups");
        let migrator = sqlx::migrate::Migrator::new(old.as_path()).await.unwrap();
        let store = Store::open_with(&db, &migrator, &backups).await.unwrap();
        let exec = |sql: &'static str| sqlx::query(sql).execute(&store.pool);
        exec("INSERT INTO projects (id, name, repo_path, base_branch, created_at, key) VALUES ('p', 'P', '/a', 'main', 1, 'P')")
            .await
            .unwrap();
        exec("INSERT INTO workspaces (id, name, project_id, position, created_at) VALUES ('w', 'P', 'p', 0, 1)")
            .await
            .unwrap();
        exec(
            "INSERT INTO sessions (id, project_id, kind, title, cwd, status, created_at) \
             VALUES ('s', 'p', 'shell', 'Shell', '/a', 'running', 1)",
        )
        .await
        .unwrap();
        exec(
            "INSERT INTO runs (id, project_id, branch, name, kind, state, cwd, created_at, updated_at) \
             VALUES ('r', 'p', 'main', 'pdt', 'run', 'ready', '/a', 1, 1)",
        )
        .await
        .unwrap();
        exec(
            "INSERT INTO workspace_tabs (id, workspace_id, session_id, run_id, position, created_at) \
             VALUES ('ts', 'w', 's', NULL, 1, 5), ('tr', 'w', NULL, 'r', 0, 6)",
        )
        .await
        .unwrap();
        store.close().await;

        let store = Store::open_with(&db, &crate::migrate::MIGRATOR, &backups).await.unwrap();
        let w = store.get_workspace("w").await.unwrap().unwrap();
        type Row<'a> = (&'a str, Option<&'a str>, Option<&'a str>, Option<&'a str>, u32);
        let tabs: Vec<Row> = w
            .tabs
            .iter()
            .map(|t| (t.id.as_str(), t.session_id.as_deref(), t.run_id.as_deref(), t.ticket_id.as_deref(), t.position))
            .collect();
        assert_eq!(tabs, [("tr", None, Some("r"), None, 0), ("ts", Some("s"), None, None, 1)]);
        // Still one tab per session, and the tabs still go with their session and workspace.
        let dup = sqlx::query(
            "INSERT INTO workspace_tabs (id, workspace_id, session_id, position, created_at) VALUES ('x', 'w', 's', 2, 1)",
        )
        .execute(&store.pool)
        .await;
        assert!(dup.is_err());
        store.delete_session("s").await.unwrap();
        assert_eq!(store.get_workspace("w").await.unwrap().unwrap().tabs.len(), 1);
        let idx: Vec<(String,)> =
            sqlx::query_as("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'workspace_tabs'")
                .fetch_all(&store.pool)
                .await
                .unwrap();
        assert!(idx.iter().any(|(n,)| n == "workspace_tabs_by_workspace"), "{idx:?}");
    }
}
