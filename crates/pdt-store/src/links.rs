//! Remote projects' links (PDT-631): on a remote instance, each remote project's link to its owner
//! (`remote_projects`); on the owner, the instances joined to each of its projects (`remote_instances`). Keys'
//! private halves are never here: they are in the keychain.

use crate::{PROJECT_COLUMNS, ProjectRow, Store, StoreError, to_project};
use pdt_core::link::{InstanceFacts, OwnerLink, RemoteInstance};
use pdt_core::project::Project;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const OWNER_LINK_COLUMNS: &str = "project_id, link_id, owner_project_id, owner_instance_id, owner_name, owner_url, \
                                  owner_version, owner_public_key, repo_url, paused, last_contact_at, created_at, \
                                  checkout_path";

fn to_owner_link(r: &SqliteRow) -> Result<OwnerLink, StoreError> {
    Ok(OwnerLink {
        project_id: r.try_get("project_id")?,
        link_id: r.try_get("link_id")?,
        owner_project_id: r.try_get("owner_project_id")?,
        owner_instance_id: r.try_get("owner_instance_id")?,
        owner_name: r.try_get("owner_name")?,
        owner_url: r.try_get("owner_url")?,
        owner_version: r.try_get("owner_version")?,
        owner_public_key: r.try_get("owner_public_key")?,
        repo_url: r.try_get("repo_url")?,
        checkout_path: r.try_get("checkout_path")?,
        paused: r.try_get::<i64, _>("paused")? != 0,
        last_contact_at: r.try_get("last_contact_at")?,
        created_at: r.try_get("created_at")?,
    })
}

const INSTANCE_COLUMNS: &str = "link_id, project_id, instance_id, name, url, public_key, version, protocol, facts, \
                                paused, last_seen_at, last_error, created_at";

fn to_instance(r: &SqliteRow) -> Result<RemoteInstance, StoreError> {
    let protocol: i64 = r.try_get("protocol")?;
    Ok(RemoteInstance {
        link_id: r.try_get("link_id")?,
        project_id: r.try_get("project_id")?,
        instance_id: r.try_get("instance_id")?,
        name: r.try_get("name")?,
        url: r.try_get("url")?,
        public_key: r.try_get("public_key")?,
        version: r.try_get("version")?,
        protocol: u32::try_from(protocol).unwrap_or(0),
        // Facts this build can't read are no facts (rather than a list that won't load).
        facts: r.try_get::<Option<String>, _>("facts")?.and_then(|j| serde_json::from_str(&j).ok()),
        paused: r.try_get::<i64, _>("paused")? != 0,
        last_seen_at: r.try_get("last_seen_at")?,
        last_error: r.try_get("last_error")?,
        created_at: r.try_get("created_at")?,
    })
}

/// What a heartbeat found: an answer (with what it said), or why there was none.
pub enum Heard<'a> {
    Answer { name: &'a str, version: &'a str, protocol: u32, facts: &'a InstanceFacts, paused: bool, at: i64 },
    Failed { error: &'a str, protocol: Option<(u32, &'a str)> },
}

impl Store {
    /// Adds a remote project (kind `remote`) and its link to the owner, in one transaction. A remote project has no
    /// workspace: nothing runs in it from this instance's own UI.
    pub async fn insert_remote_project(&self, p: &Project, link: &OwnerLink) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        sqlx::query(
            "INSERT INTO projects (id, name, repo_path, base_branch, created_at, key, kind) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.repo_path)
        .bind(&p.base_branch)
        .bind(p.created_at)
        .bind(&p.key)
        .bind(pdt_core::project::ProjectKind::Remote.as_str())
        .execute(&mut *tx)
        .await?;
        let sql = format!(
            "INSERT INTO remote_projects ({OWNER_LINK_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&p.id)
            .bind(&link.link_id)
            .bind(&link.owner_project_id)
            .bind(&link.owner_instance_id)
            .bind(&link.owner_name)
            .bind(&link.owner_url)
            .bind(&link.owner_version)
            .bind(&link.owner_public_key)
            .bind(&link.repo_url)
            .bind(link.paused)
            .bind(link.last_contact_at)
            .bind(link.created_at)
            .bind(&link.checkout_path)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// This instance's remote projects, with their links, oldest first.
    pub async fn list_remote_projects(&self) -> Result<Vec<(Project, OwnerLink)>, StoreError> {
        let cols: String = PROJECT_COLUMNS.split(", ").map(|c| format!("p.{c}")).collect::<Vec<_>>().join(", ");
        let sql = format!(
            "SELECT {cols}, {} FROM projects p JOIN remote_projects r ON r.project_id = p.id \
             WHERE p.kind = 'remote' ORDER BY p.created_at, p.id",
            OWNER_LINK_COLUMNS.split(", ").map(|c| format!("r.{c}")).collect::<Vec<_>>().join(", ")
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter()
            .map(|r| {
                let project: ProjectRow = (
                    r.try_get("id")?,
                    r.try_get("name")?,
                    r.try_get("repo_path")?,
                    r.try_get("base_branch")?,
                    r.try_get("created_at")?,
                    r.try_get("key")?,
                    r.try_get("kind")?,
                );
                Ok((to_project(project), to_owner_link(r)?))
            })
            .collect()
    }

    /// A remote project, by this instance's id of it.
    pub async fn get_remote_project(&self, id: &str) -> Result<Option<Project>, StoreError> {
        let sql = format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ? AND kind = 'remote'");
        let row: Option<ProjectRow> =
            sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(to_project))
    }

    /// A remote project's link, by this instance's project id.
    pub async fn get_owner_link(&self, project_id: &str) -> Result<Option<OwnerLink>, StoreError> {
        let sql = format!("SELECT {OWNER_LINK_COLUMNS} FROM remote_projects WHERE project_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_owner_link).transpose()
    }

    /// A remote project's link, by the link's id.
    pub async fn owner_link_by_id(&self, link_id: &str) -> Result<Option<OwnerLink>, StoreError> {
        let sql = format!("SELECT {OWNER_LINK_COLUMNS} FROM remote_projects WHERE link_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(link_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_owner_link).transpose()
    }

    /// The owner reached this instance on `link_id` at `at`, as `owner_version`.
    pub async fn owner_contact(&self, link_id: &str, at: i64, owner_version: Option<&str>) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE remote_projects SET last_contact_at = ?, owner_version = COALESCE(?, owner_version) \
             WHERE link_id = ?",
        )
        .bind(at)
        .bind(owner_version)
        .bind(link_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Pauses or resumes a remote project here. False when there is no such remote project.
    pub async fn set_remote_project_paused(&self, project_id: &str, paused: bool) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE remote_projects SET paused = ? WHERE project_id = ?")
            .bind(paused)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Sets (None: back to the default) where this instance keeps a remote project's checkout.
    pub async fn set_remote_project_checkout(&self, project_id: &str, path: Option<&str>) -> Result<bool, StoreError> {
        let r = sqlx::query("UPDATE remote_projects SET checkout_path = ? WHERE project_id = ?")
            .bind(path)
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// How far the owner has read a joined instance's session events: the instance's boot they were numbered in, and
    /// the last one taken.
    pub async fn events_cursor(&self, link_id: &str) -> Result<(Option<String>, i64), StoreError> {
        let row: Option<(Option<String>, i64)> =
            sqlx::query_as("SELECT events_boot, events_after FROM remote_instances WHERE link_id = ?")
                .bind(link_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.unwrap_or((None, 0)))
    }

    pub async fn set_events_cursor(&self, link_id: &str, boot: &str, after: i64) -> Result<(), StoreError> {
        sqlx::query("UPDATE remote_instances SET events_boot = ?, events_after = ? WHERE link_id = ?")
            .bind(boot)
            .bind(after)
            .bind(link_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Joins an instance to one of this instance's projects.
    pub async fn insert_remote_instance(&self, i: &RemoteInstance) -> Result<(), StoreError> {
        let sql =
            format!("INSERT INTO remote_instances ({INSTANCE_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(&i.link_id)
            .bind(&i.project_id)
            .bind(&i.instance_id)
            .bind(&i.name)
            .bind(&i.url)
            .bind(&i.public_key)
            .bind(&i.version)
            .bind(i64::from(i.protocol))
            .bind(i.facts.as_ref().map(|f| serde_json::to_string(f).expect("facts serialize")))
            .bind(i.paused)
            .bind(i.last_seen_at)
            .bind(&i.last_error)
            .bind(i.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The instances joined to `project_id`, oldest first; every project's when None.
    pub async fn list_remote_instances(&self, project_id: Option<&str>) -> Result<Vec<RemoteInstance>, StoreError> {
        let sql = format!(
            "SELECT {INSTANCE_COLUMNS} FROM remote_instances WHERE (?1 IS NULL OR project_id = ?1) \
             ORDER BY created_at, link_id"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_instance).collect()
    }

    pub async fn get_remote_instance(&self, link_id: &str) -> Result<Option<RemoteInstance>, StoreError> {
        let sql = format!("SELECT {INSTANCE_COLUMNS} FROM remote_instances WHERE link_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(link_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_instance).transpose()
    }

    /// Records a heartbeat's outcome for `link_id`.
    pub async fn record_heartbeat(&self, link_id: &str, heard: Heard<'_>) -> Result<(), StoreError> {
        match heard {
            Heard::Answer { name, version, protocol, facts, paused, at } => {
                sqlx::query(
                    "UPDATE remote_instances SET name = ?, version = ?, protocol = ?, facts = ?, paused = ?, \
                     last_seen_at = ?, last_error = NULL WHERE link_id = ?",
                )
                .bind(name)
                .bind(version)
                .bind(i64::from(protocol))
                .bind(serde_json::to_string(facts).expect("facts serialize"))
                .bind(paused)
                .bind(at)
                .bind(link_id)
                .execute(&self.pool)
                .await?;
            }
            Heard::Failed { error, protocol } => {
                let (protocol, version) = protocol.map_or((None, None), |(p, v)| (Some(i64::from(p)), Some(v)));
                sqlx::query(
                    "UPDATE remote_instances SET last_error = ?, protocol = COALESCE(?, protocol), \
                     version = COALESCE(?, version) WHERE link_id = ?",
                )
                .bind(error)
                .bind(protocol)
                .bind(version)
                .bind(link_id)
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Removes a joined instance. False when there was none.
    pub async fn delete_remote_instance(&self, link_id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM remote_instances WHERE link_id = ?").bind(link_id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pdt_core::project::ProjectKind;

    pub(crate) fn remote(id: &str) -> (Project, OwnerLink) {
        let p = Project {
            id: id.into(),
            name: "Val".into(),
            repo_path: format!("remote:{id}"),
            base_branch: "main".into(),
            created_at: 1,
            key: "VAL".into(),
            forge: None,
            kind: ProjectKind::Remote,
        };
        let link = OwnerLink {
            project_id: id.into(),
            link_id: format!("l-{id}"),
            owner_project_id: "owner-p".into(),
            owner_instance_id: "owner-i".into(),
            owner_name: "studio".into(),
            owner_url: "https://studio.example.ts.net".into(),
            owner_version: "1.0.0".into(),
            owner_public_key: "pk".into(),
            repo_url: Some("git@github.com:o/val.git".into()),
            checkout_path: None,
            paused: false,
            last_contact_at: None,
            created_at: 1,
        };
        (p, link)
    }

    #[tokio::test]
    async fn remote_projects_are_kept_apart_from_this_instances_own() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let (p, link) = remote("r");
        store.insert_remote_project(&p, &link).await.unwrap();
        assert!(store.list_projects().await.unwrap().is_empty(), "never listed with this instance's own");
        assert!(store.list_workspaces().await.unwrap().is_empty(), "no workspace");
        assert_eq!(store.get_project("r").await.unwrap(), None, "never found as one of this instance's own");
        assert_eq!(store.get_remote_project("r").await.unwrap(), Some(p.clone()));
        let listed = store.list_remote_projects().await.unwrap();
        assert_eq!(listed, vec![(p.clone(), link.clone())]);
        assert_eq!(store.owner_link_by_id("l-r").await.unwrap(), Some(link.clone()));

        store.owner_contact("l-r", 50, Some("1.1.0")).await.unwrap();
        assert!(store.set_remote_project_paused("r", true).await.unwrap());
        let got = store.get_owner_link("r").await.unwrap().unwrap();
        assert_eq!((got.last_contact_at, got.owner_version.as_str(), got.paused), (Some(50), "1.1.0", true));

        // The same owner's project can't be joined twice.
        let (mut again, mut link2) = remote("r2");
        link2.link_id = "l-other".into();
        again.key = "VAL2".into();
        assert!(matches!(store.insert_remote_project(&again, &link2).await, Err(StoreError::Conflict(_))));

        assert!(store.delete_project("r").await.unwrap());
        assert_eq!(store.get_owner_link("r").await.unwrap(), None, "the link goes with the project");
    }

    #[tokio::test]
    async fn joined_instances_record_heartbeats() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let p = Project {
            id: "p".into(),
            name: "Val".into(),
            repo_path: tmp.path().to_string_lossy().into_owned(),
            base_branch: "main".into(),
            created_at: 1,
            key: "VAL".into(),
            forge: None,
            kind: ProjectKind::Repo,
        };
        store.insert_project(&p).await.unwrap();
        let i = RemoteInstance {
            link_id: "l".into(),
            project_id: "p".into(),
            instance_id: "win".into(),
            name: "win-box".into(),
            url: "https://win-box.example.ts.net".into(),
            public_key: "pk".into(),
            version: "1.0.0".into(),
            protocol: 1,
            facts: None,
            paused: false,
            last_seen_at: None,
            last_error: None,
            created_at: 2,
        };
        store.insert_remote_instance(&i).await.unwrap();
        let dup = RemoteInstance { link_id: "l2".into(), ..i.clone() };
        assert!(matches!(store.insert_remote_instance(&dup).await, Err(StoreError::Conflict(_))), "joined once");

        let facts = InstanceFacts { os: "windows".into(), cpus: 8, ..Default::default() };
        let answer = Heard::Answer { name: "win", version: "1.1.0", protocol: 1, facts: &facts, paused: true, at: 9 };
        store.record_heartbeat("l", answer).await.unwrap();
        let got = store.get_remote_instance("l").await.unwrap().unwrap();
        assert_eq!((got.name.as_str(), got.version.as_str(), got.paused), ("win", "1.1.0", true));
        assert_eq!((got.facts, got.last_seen_at), (Some(facts), Some(9)));

        store.record_heartbeat("l", Heard::Failed { error: "offline", protocol: Some((2, "2.0.0")) }).await.unwrap();
        let got = store.get_remote_instance("l").await.unwrap().unwrap();
        assert_eq!((got.last_error.as_deref(), got.protocol, got.last_seen_at), (Some("offline"), 2, Some(9)));

        assert_eq!(store.list_remote_instances(Some("p")).await.unwrap().len(), 1);
        assert_eq!(store.list_remote_instances(Some("q")).await.unwrap().len(), 0);
        assert_eq!(store.list_remote_instances(None).await.unwrap().len(), 1);
        assert!(store.delete_remote_instance("l").await.unwrap());
        assert!(!store.delete_remote_instance("l").await.unwrap());
    }
}
