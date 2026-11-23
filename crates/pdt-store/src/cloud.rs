//! Cloud instances (PDT-638), on the owner: what the user set for a joined instance that is a cloud VM, what the
//! owner's check keeps between rounds, and the cost meter.

use crate::{Store, StoreError};
use pdt_core::cloud::{CloudSetup, CloudState, CloudUsage, Power};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

/// A joined instance's cloud setup and state.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudInstance {
    pub link_id: String,
    pub setup: CloudSetup,
    pub state: CloudState,
}

const COLUMNS: &str = "link_id, connection, machine, idle_minutes, hourly_rate, power, checked_at, last_busy_at, \
                       start_requested_at, stop_requested_at, last_error";

fn to_cloud(r: &SqliteRow) -> Result<CloudInstance, StoreError> {
    Ok(CloudInstance {
        link_id: r.try_get("link_id")?,
        setup: CloudSetup {
            connection: r.try_get("connection")?,
            machine: r.try_get("machine")?,
            idle_minutes: u32::try_from(r.try_get::<i64, _>("idle_minutes")?).unwrap_or(0),
            hourly_rate: r.try_get("hourly_rate")?,
        },
        state: CloudState {
            power: r.try_get::<Option<String>, _>("power")?.map(|p| Power::parse(&p)),
            checked_at: r.try_get("checked_at")?,
            last_busy_at: r.try_get("last_busy_at")?,
            start_requested_at: r.try_get("start_requested_at")?,
            stop_requested_at: r.try_get("stop_requested_at")?,
            last_error: r.try_get("last_error")?,
        },
    })
}

impl Store {
    /// Every joined instance that is a cloud VM (of `project_id`'s, or all when None).
    pub async fn list_cloud_instances(&self, project_id: Option<&str>) -> Result<Vec<CloudInstance>, StoreError> {
        let sql = format!(
            "SELECT {COLUMNS} FROM cloud_instances WHERE ?1 IS NULL OR link_id IN \
             (SELECT link_id FROM remote_instances WHERE project_id = ?1) ORDER BY created_at, link_id"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(project_id).fetch_all(&self.pool).await?;
        rows.iter().map(to_cloud).collect()
    }

    pub async fn get_cloud_instance(&self, link_id: &str) -> Result<Option<CloudInstance>, StoreError> {
        let sql = format!("SELECT {COLUMNS} FROM cloud_instances WHERE link_id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(link_id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_cloud).transpose()
    }

    /// Sets a joined instance's cloud setup (None: it is no cloud instance any more), keeping its state and meter.
    pub async fn set_cloud_setup(&self, link_id: &str, setup: Option<&CloudSetup>, now: i64) -> Result<(), StoreError> {
        match setup {
            Some(s) => {
                sqlx::query(
                    "INSERT INTO cloud_instances (link_id, connection, machine, idle_minutes, hourly_rate, created_at) \
                     VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (link_id) DO UPDATE SET connection = excluded.connection, \
                     machine = excluded.machine, idle_minutes = excluded.idle_minutes, \
                     hourly_rate = excluded.hourly_rate",
                )
                .bind(link_id)
                .bind(&s.connection)
                .bind(&s.machine)
                .bind(i64::from(s.idle_minutes))
                .bind(s.hourly_rate)
                .bind(now)
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query("DELETE FROM cloud_instances WHERE link_id = ?").bind(link_id).execute(&self.pool).await?;
            }
        }
        Ok(())
    }

    /// Keeps what a check of a cloud instance found.
    pub async fn set_cloud_state(&self, link_id: &str, s: &CloudState) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE cloud_instances SET power = ?, checked_at = ?, last_busy_at = ?, start_requested_at = ?, \
             stop_requested_at = ?, last_error = ? WHERE link_id = ?",
        )
        .bind(s.power.map(Power::as_str))
        .bind(s.checked_at)
        .bind(s.last_busy_at)
        .bind(s.start_requested_at)
        .bind(s.stop_requested_at)
        .bind(&s.last_error)
        .bind(link_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Adds `ms` of running, and what it cost, to an instance's meter for `month`.
    pub async fn add_cloud_usage(&self, link_id: &str, month: &str, ms: i64, cost: f64) -> Result<(), StoreError> {
        if ms <= 0 {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO cloud_usage (link_id, month, running_ms, cost) VALUES (?, ?, ?, ?) \
             ON CONFLICT (link_id, month) DO UPDATE SET running_ms = running_ms + excluded.running_ms, \
             cost = cost + excluded.cost",
        )
        .bind(link_id)
        .bind(month)
        .bind(ms)
        .bind(cost)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// An instance's meter, newest month first.
    pub async fn cloud_usage(&self, link_id: &str) -> Result<Vec<CloudUsage>, StoreError> {
        let rows = sqlx::query("SELECT month, running_ms, cost FROM cloud_usage WHERE link_id = ? ORDER BY month DESC")
            .bind(link_id)
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|r| {
                Ok(CloudUsage {
                    month: r.try_get("month")?,
                    hours: r.try_get::<i64, _>("running_ms")? as f64 / 3_600_000.0,
                    cost: r.try_get("cost")?,
                })
            })
            .collect()
    }

    /// Whether sessions or runs are live on joined instance `instance_id` of `project_id`.
    pub async fn instance_busy(&self, project_id: &str, instance_id: &str) -> Result<bool, StoreError> {
        let busy: i64 = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM sessions WHERE project_id = ?1 AND instance_id = ?2 AND status = 'running') \
             OR EXISTS (SELECT 1 FROM runs WHERE project_id = ?1 AND instance_id = ?2 \
                        AND state IN ('setting_up', 'starting', 'ready', 'running'))",
        )
        .bind(project_id)
        .bind(instance_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(busy != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::link::RemoteInstance;

    async fn joined(store: &Store, dir: &std::path::Path) -> RemoteInstance {
        let p = pdt_core::project::Project {
            id: "p".into(),
            name: "Val".into(),
            repo_path: dir.to_string_lossy().into_owned(),
            base_branch: "main".into(),
            created_at: 1,
            key: "VAL".into(),
            forge: None,
            kind: pdt_core::project::ProjectKind::Repo,
        };
        store.insert_project(&p).await.unwrap();
        let i = RemoteInstance {
            link_id: "l1".into(),
            project_id: p.id.clone(),
            instance_id: "w".into(),
            name: "win-cloud".into(),
            url: "https://w".into(),
            public_key: "k".into(),
            version: "1".into(),
            protocol: 1,
            facts: None,
            paused: false,
            last_seen_at: None,
            last_error: None,
            secrets: vec![],
            cap: None,
            remote_cap: None,
            created_at: 1,
        };
        store.insert_remote_instance(&i).await.unwrap();
        i
    }

    #[tokio::test]
    async fn a_cloud_setup_keeps_its_state_and_meter_and_goes_with_its_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let i = joined(&store, tmp.path()).await;
        let setup = CloudSetup { connection: "aws".into(), machine: "i-1".into(), idle_minutes: 30, hourly_rate: 0.5 };
        store.set_cloud_setup("l1", Some(&setup), 5).await.unwrap();
        let state = CloudState { power: Some(Power::Running), checked_at: Some(9), ..Default::default() };
        store.set_cloud_state("l1", &state).await.unwrap();
        let again = CloudSetup { idle_minutes: 10, ..setup.clone() };
        store.set_cloud_setup("l1", Some(&again), 6).await.unwrap();
        let got = store.get_cloud_instance("l1").await.unwrap().unwrap();
        assert_eq!((got.setup, got.state), (again, state));
        assert_eq!(store.list_cloud_instances(Some(&i.project_id)).await.unwrap().len(), 1);
        assert!(store.list_cloud_instances(Some("other")).await.unwrap().is_empty());

        store.add_cloud_usage("l1", "2026-11", 1_800_000, 0.25).await.unwrap();
        store.add_cloud_usage("l1", "2026-11", 1_800_000, 0.25).await.unwrap();
        store.add_cloud_usage("l1", "2026-12", 0, 0.0).await.unwrap();
        let usage = store.cloud_usage("l1").await.unwrap();
        assert_eq!(usage, vec![CloudUsage { month: "2026-11".into(), hours: 1.0, cost: 0.5 }]);

        assert!(!store.instance_busy(&i.project_id, "w").await.unwrap());
        store.delete_remote_instance("l1").await.unwrap();
        assert!(store.get_cloud_instance("l1").await.unwrap().is_none());
        assert!(store.cloud_usage("l1").await.unwrap().is_empty());
    }
}
