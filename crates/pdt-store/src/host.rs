use crate::{Store, StoreError};
use pdt_core::host::{CredentialPolicy, HostSettings};
use pdt_core::integration::IntegrationSettings;
use pdt_core::policy::{AutoStart, AutoStartOrder, MergeBy, ProjectPolicy, ResolveConflicts, TriageApproval};
use std::collections::BTreeMap;

const HOST_KEY: &str = "host";

/// `project_settings` columns: run overrides, credential policy, agent instructions, merge through PRs, permission
/// mode, and the policies (merge by, require approval, running-attempt cap, auto-start, approval-exempt labels, the PM
/// may act), the integrations' settings (JSON), the start delay, green CI, auto-start order and triage approval. (0027's integration columns are
/// unused since the integration settings took them in, PDT-174.)
type ProjectSettingsRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
    String,
    bool,
    Option<i64>,
    String,
    String,
    bool,
    String,
    i64,
    bool,
    String,
    String,
);

/// A project's settings on this host (overrides of the repo's run configuration, credential policy).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectSettings {
    /// TOML in the `.pdt/project.toml` schema, merged over the repo's file (spec §4.1).
    pub run_overrides: Option<String>,
    /// None means "use the host's policy".
    pub agent_credentials: Option<CredentialPolicy>,
    /// Instructions given to every agent session in the project (on top of the repo's CLAUDE.md / AGENTS.md).
    pub agent_instructions: Option<String>,
    /// Merge… only through the attempt's pull request (review spec §2.7): never a local merge.
    pub merge_via_pr: bool,
    /// The permission mode every persona's session in the project runs with, over the persona's own (Phase 7b §3).
    pub agent_permission_mode: Option<pdt_core::conversation::PermissionMode>,
    /// Who may merge, approvals, the running-attempt cap and auto-start (Phase 7c §4).
    pub policy: ProjectPolicy,
    /// Its settings for each integration it is linked to, by the integration's id (PDT-174): `connection` and the
    /// fields the integration's project settings declare.
    pub integrations: BTreeMap<String, IntegrationSettings>,
}

impl ProjectSettings {
    /// Its settings for integration `id` as `T`, if it is linked and they read as one.
    pub fn integration<T: serde::de::DeserializeOwned>(&self, id: &str) -> Option<T> {
        serde_json::from_value(serde_json::Value::Object(self.integrations.get(id)?.clone())).ok()
    }
}

/// Where the integrations this instance has enabled are kept (PDT-173): a JSON list of their ids.
const ENABLED_INTEGRATIONS: &str = "integrations.enabled";
/// PDT's own integrations this instance turned off (PDT-251: GitHub): a JSON list of their ids.
const OWN_INTEGRATIONS_OFF: &str = "integrations.own_off";

impl Store {
    pub async fn get_host_settings(&self) -> Result<HostSettings, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM host_settings WHERE key = ?")
            .bind(HOST_KEY)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            None => Ok((*self.host_defaults).clone()),
            Some((v,)) => serde_json::from_str(&v).map_err(|e| StoreError::Corrupt(format!("host settings: {e}"))),
        }
    }

    pub async fn put_host_settings(&self, s: &HostSettings) -> Result<(), StoreError> {
        let value = serde_json::to_string(s).expect("host settings serialize");
        sqlx::query(
            "INSERT INTO host_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(HOST_KEY)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A value of its own under `key` in `host_settings` (a sensor's baseline: `sensor.<name>.<key>`).
    pub async fn get_host_value(&self, key: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM host_settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|(v,)| serde_json::from_str(&v).ok()))
    }

    pub async fn put_host_value(&self, key: &str, value: &serde_json::Value) -> Result<(), StoreError> {
        assert_ne!(key, HOST_KEY, "the host settings have their own setter");
        sqlx::query(
            "INSERT INTO host_settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(value.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_host_value(&self, key: &str) -> Result<(), StoreError> {
        sqlx::query("DELETE FROM host_settings WHERE key = ? AND key != ?")
            .bind(key)
            .bind(HOST_KEY)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The keys of the host values that start with `prefix` (taken literally), sorted.
    pub async fn host_keys_with_prefix(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT key FROM host_settings WHERE substr(key, 1, length(?)) = ? AND key != ? ORDER BY key",
        )
        .bind(prefix)
        .bind(prefix)
        .bind(HOST_KEY)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(k,)| k).collect())
    }

    /// The integrations this instance has enabled (PDT-173), by id.
    pub async fn enabled_integrations(&self) -> Result<Vec<String>, StoreError> {
        let v = self.get_host_value(ENABLED_INTEGRATIONS).await?;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }

    pub async fn set_enabled_integrations(&self, ids: &[String]) -> Result<(), StoreError> {
        self.put_host_value(ENABLED_INTEGRATIONS, &serde_json::json!(ids)).await
    }

    /// PDT's own integrations this instance turned off (PDT-251), by id; none until the user turns one off.
    pub async fn own_integrations_off(&self) -> Result<Vec<String>, StoreError> {
        let v = self.get_host_value(OWN_INTEGRATIONS_OFF).await?;
        Ok(v.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())
    }

    pub async fn set_own_integrations_off(&self, ids: &[String]) -> Result<(), StoreError> {
        self.put_host_value(OWN_INTEGRATIONS_OFF, &serde_json::json!(ids)).await
    }

    /// Decides which integrations an instance that has never chosen enables (PDT-173): the ones it already uses (a
    /// connection of one of their kinds, or a workflow of the user's with one of their node kinds), so an upgraded
    /// install keeps working with no action; a new one has none.
    pub(crate) async fn backfill_enabled_integrations(&self) -> Result<(), StoreError> {
        if self.get_host_value(ENABLED_INTEGRATIONS).await?.is_some() {
            return Ok(());
        }
        let connections: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT kind FROM connections").fetch_all(&self.pool).await?;
        let nodes: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT json_extract(n.value, '$.kind') FROM workflows w \
             JOIN workflow_versions v ON v.workflow_id = w.id AND v.version = w.version, json_each(v.doc, '$.nodes') n \
             WHERE w.builtin IS NULL AND json_valid(v.doc) AND json_extract(n.value, '$.kind') IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let used = pdt_core::integration::in_use(
            connections.iter().map(|(k,)| k.as_str()),
            nodes.iter().map(|(k,)| k.as_str()),
        );
        self.set_enabled_integrations(&used.into_iter().map(String::from).collect::<Vec<_>>()).await
    }

    pub async fn get_project_settings(&self, project_id: &str) -> Result<ProjectSettings, StoreError> {
        let row: Option<ProjectSettingsRow> = sqlx::query_as(
            "SELECT run_overrides, agent_credentials, agent_instructions, merge_via_pr, agent_permission_mode, \
             merge_by, require_approval, max_running_attempts, auto_start, approval_exempt_labels, pm_acts, \
             integration_settings, start_delay_seconds, require_green_ci, auto_start_order, triage_approval \
             FROM project_settings \
             WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((
            run_overrides,
            creds,
            agent_instructions,
            merge_via_pr,
            mode,
            merge_by,
            require_approval,
            cap,
            auto,
            exempt,
            pm_acts,
            integrations,
            start_delay,
            require_green_ci,
            order,
            triage,
        )) = row
        else {
            return Ok(ProjectSettings::default());
        };
        // Past the 16 columns a row tuple can hold.
        let (resolve, hot, untagged): (String, String, bool) = sqlx::query_as(
            "SELECT resolve_conflicts, hot_areas, untagged_alongside_hot FROM project_settings WHERE project_id = ?",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await?;
        let agent_credentials = match creds {
            Some(c) => Some(
                CredentialPolicy::parse(&c).ok_or_else(|| StoreError::Corrupt(format!("credential policy {c:?}")))?,
            ),
            None => None,
        };
        let agent_permission_mode = mode.as_deref().and_then(pdt_core::conversation::PermissionMode::parse);
        let integrations = serde_json::from_str(&integrations)
            .map_err(|e| StoreError::Corrupt(format!("integration settings {integrations:?}: {e}")))?;
        Ok(ProjectSettings {
            run_overrides,
            agent_credentials,
            agent_instructions,
            merge_via_pr,
            agent_permission_mode,
            policy: ProjectPolicy {
                merge_by: MergeBy::parse(&merge_by)
                    .ok_or_else(|| StoreError::Corrupt(format!("merge by {merge_by:?}")))?,
                require_approval,
                max_running_attempts: cap.and_then(|n| u32::try_from(n).ok()),
                auto_start: AutoStart::parse(&auto)
                    .ok_or_else(|| StoreError::Corrupt(format!("auto-start {auto:?}")))?,
                auto_start_order: AutoStartOrder::parse(&order)
                    .ok_or_else(|| StoreError::Corrupt(format!("auto-start order {order:?}")))?,
                approval_exempt_labels: serde_json::from_str(&exempt)
                    .map_err(|_| StoreError::Corrupt(format!("approval-exempt labels {exempt:?}")))?,
                pm_acts,
                start_delay_seconds: u32::try_from(start_delay).unwrap_or(0),
                require_green_ci,
                triage_approval: TriageApproval::parse(&triage)
                    .ok_or_else(|| StoreError::Corrupt(format!("triage approval {triage:?}")))?,
                resolve_conflicts: ResolveConflicts::parse(&resolve)
                    .ok_or_else(|| StoreError::Corrupt(format!("resolve conflicts {resolve:?}")))?,
                hot_areas: serde_json::from_str(&hot).map_err(|_| StoreError::Corrupt(format!("hot areas {hot:?}")))?,
                untagged_alongside_hot: untagged,
            },
            integrations,
        })
    }

    pub async fn put_project_settings(&self, project_id: &str, s: &ProjectSettings) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO project_settings (project_id, run_overrides, agent_credentials, agent_instructions, \
             merge_via_pr, agent_permission_mode, merge_by, require_approval, max_running_attempts, auto_start, \
             approval_exempt_labels, pm_acts, integration_settings, start_delay_seconds, require_green_ci, \
             auto_start_order, triage_approval, resolve_conflicts, hot_areas, untagged_alongside_hot) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(project_id) DO UPDATE SET \
             run_overrides = excluded.run_overrides, agent_credentials = excluded.agent_credentials, \
             agent_instructions = excluded.agent_instructions, merge_via_pr = excluded.merge_via_pr, \
             agent_permission_mode = excluded.agent_permission_mode, merge_by = excluded.merge_by, \
             require_approval = excluded.require_approval, max_running_attempts = excluded.max_running_attempts, \
             auto_start = excluded.auto_start, approval_exempt_labels = excluded.approval_exempt_labels, \
             pm_acts = excluded.pm_acts, integration_settings = excluded.integration_settings, \
             start_delay_seconds = excluded.start_delay_seconds, require_green_ci = excluded.require_green_ci, \
             auto_start_order = excluded.auto_start_order, triage_approval = excluded.triage_approval, \
             resolve_conflicts = excluded.resolve_conflicts, hot_areas = excluded.hot_areas, \
             untagged_alongside_hot = excluded.untagged_alongside_hot",
        )
        .bind(project_id)
        .bind(&s.run_overrides)
        .bind(s.agent_credentials.map(CredentialPolicy::as_str))
        .bind(&s.agent_instructions)
        .bind(s.merge_via_pr)
        .bind(s.agent_permission_mode.map(pdt_core::conversation::PermissionMode::as_str))
        .bind(s.policy.merge_by.as_str())
        .bind(s.policy.require_approval)
        .bind(s.policy.max_running_attempts.map(i64::from))
        .bind(s.policy.auto_start.as_str())
        .bind(serde_json::to_string(&s.policy.approval_exempt_labels).expect("a list of labels"))
        .bind(s.policy.pm_acts)
        .bind(serde_json::to_string(&s.integrations).expect("JSON objects"))
        .bind(i64::from(s.policy.start_delay_seconds))
        .bind(s.policy.require_green_ci)
        .bind(s.policy.auto_start_order.as_str())
        .bind(s.policy.triage_approval.as_str())
        .bind(s.policy.resolve_conflicts.as_str())
        .bind(serde_json::to_string(&s.policy.hot_areas).expect("a list of areas"))
        .bind(s.policy.untagged_alongside_hot)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

impl Store {
    /// A project's automatic worktree cleanup (defaults until it is set).
    pub async fn get_project_storage(
        &self,
        project_id: &str,
    ) -> Result<pdt_core::storage::StorageSettings, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT storage FROM project_settings WHERE project_id = ?")
            .bind(project_id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            None => Ok(Default::default()),
            Some((v,)) => serde_json::from_str(&v).map_err(|e| StoreError::Corrupt(format!("storage settings: {e}"))),
        }
    }

    pub async fn put_project_storage(
        &self,
        project_id: &str,
        s: &pdt_core::storage::StorageSettings,
    ) -> Result<(), StoreError> {
        // The row gets the other settings as they read now, so a project with none keeps its defaults.
        let current = self.get_project_settings(project_id).await?;
        self.put_project_settings(project_id, &current).await?;
        sqlx::query("UPDATE project_settings SET storage = ? WHERE project_id = ?")
            .bind(serde_json::to_string(s).expect("storage settings serialize"))
            .bind(project_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::worktrees::tests::store_with_project;
    use crate::{ProjectSettings, StoreError};
    use pdt_core::host::{CredentialPolicy, HostSettings, UpdatePolicy};
    use serde_json::json;

    #[tokio::test]
    async fn project_storage_defaults_then_roundtrips_and_keeps_the_other_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let defaults = pdt_core::storage::StorageSettings::default();
        assert_eq!(store.get_project_storage("p").await.unwrap(), defaults);
        let s = pdt_core::storage::StorageSettings { clean_after_days: 0, remove_after_days: 9, ..defaults };
        store.put_project_storage("p", &s).await.unwrap();
        assert_eq!(store.get_project_storage("p").await.unwrap(), s);
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        assert!(store.put_project_storage("missing", &s).await.is_err());
    }

    #[tokio::test]
    async fn host_settings_default_then_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.get_host_settings().await.unwrap(), HostSettings::default());
        let s = HostSettings {
            update_policy: UpdatePolicy::AutoWhenIdle,
            owner_login: Some("me@example.com".into()),
            ..HostSettings::default()
        };
        store.put_host_settings(&s).await.unwrap();
        store.put_host_settings(&s).await.unwrap(); // upsert
        assert_eq!(store.get_host_settings().await.unwrap(), s);
    }

    /// PDT-283: a host saved on the experimental channel is on the nightly one, and is saved so next time.
    #[tokio::test]
    async fn a_stored_experimental_channel_reads_as_nightly() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        sqlx::query("INSERT INTO host_settings (key, value) VALUES (?, ?)")
            .bind(super::HOST_KEY)
            .bind(r#"{"updateChannel":"experimental","updatePolicy":"auto_when_idle"}"#)
            .execute(&store.pool)
            .await
            .unwrap();
        let s = store.get_host_settings().await.unwrap();
        assert_eq!(s.update_channel, pdt_core::host::UpdateChannel::Nightly);
        assert_eq!(s.update_policy, UpdatePolicy::AutoWhenIdle);
        store.put_host_settings(&s).await.unwrap();
        let (raw,): (String,) = sqlx::query_as("SELECT value FROM host_settings WHERE key = ?")
            .bind(super::HOST_KEY)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        assert!(raw.contains(r#""updateChannel":"nightly""#), "{raw}");
    }

    #[tokio::test]
    async fn unset_host_settings_use_the_given_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let dev = HostSettings::for_profile(pdt_core::brand::Profile::Dev);
        let store = store_with_project(tmp.path()).await.with_host_defaults(dev.clone());
        assert_eq!(store.get_host_settings().await.unwrap(), dev);
    }

    #[tokio::test]
    async fn host_values_sit_beside_the_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.get_host_value("sensor.x.y").await.unwrap(), None);
        store.put_host_value("sensor.x.y", &serde_json::json!({"n": 3})).await.unwrap();
        store.put_host_value("sensor.x.y", &serde_json::json!({"n": 4})).await.unwrap();
        assert_eq!(store.get_host_value("sensor.x.y").await.unwrap(), Some(serde_json::json!({"n": 4})));
        assert_eq!(store.get_host_settings().await.unwrap(), HostSettings::default(), "the settings are untouched");
        // A prefix is literal: `_` matches only itself.
        store.put_host_value("sensor.x_z", &serde_json::json!(1)).await.unwrap();
        store.put_host_value("sensor.xaz", &serde_json::json!(1)).await.unwrap();
        assert_eq!(store.host_keys_with_prefix("sensor.x_").await.unwrap(), ["sensor.x_z"]);
        assert_eq!(store.host_keys_with_prefix("sensor.x").await.unwrap(), ["sensor.x.y", "sensor.x_z", "sensor.xaz"]);
        store.delete_host_value("sensor.x.y").await.unwrap();
        assert_eq!(store.get_host_value("sensor.x.y").await.unwrap(), None);
    }

    /// PDT-173: an upgraded install enables the integrations it already uses, with no action from the user; a new
    /// one enables none, and a choice once made is kept.
    #[tokio::test]
    async fn an_upgraded_install_enables_the_integrations_it_uses() {
        use pdt_core::workflow::api::{Author, Connection, Scope, Source, Workflow};
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("pdt.db");
        let store = crate::Store::open(&db).await.unwrap();
        assert!(store.enabled_integrations().await.unwrap().is_empty(), "a new install enables none");
        let conn = |id: &str, kind: &str| Connection {
            id: id.into(),
            name: id.into(),
            kind: kind.into(),
            url: String::new(),
            config: json!({}),
            has_secret: true,
            created_at: 1,
            updated_at: 1,
            last_error: None,
            last_error_at: None,
        };
        store.insert_connection(&conn("media", "scryer")).await.unwrap();
        store.insert_connection(&conn("alerts", "discord_webhook")).await.unwrap();
        store.insert_connection(&conn("api", "http")).await.unwrap();
        let workflow = |id: &str, builtin: Option<&str>| Workflow {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            scope: Scope::Host,
            project_id: None,
            enabled: true,
            builtin: builtin.map(String::from),
            source: Source::Host,
            repo_file: None,
            version: 1,
            proposed: None,
            created_at: 1,
            updated_at: 1,
        };
        let doc = |kind: &str| {
            serde_json::from_value(json!({"name": "w", "nodes": [{"id": "a", "kind": kind, "version": 1}]})).unwrap()
        };
        store.insert_workflow(&workflow("w1", None), &doc("plex.search"), Author::User, "").await.unwrap();
        // A built-in PDT installed is not the user's use of its integration.
        store
            .insert_workflow(&workflow("w2", Some("jira-import")), &doc("jira.import"), Author::Builtin, "")
            .await
            .unwrap();
        // Before this version there was no setting: an upgrade.
        store.delete_host_value(super::ENABLED_INTEGRATIONS).await.unwrap();
        store.close().await;

        let store = crate::Store::open(&db).await.unwrap();
        assert_eq!(store.enabled_integrations().await.unwrap(), ["discord", "scryer", "plex"]);
        store.set_enabled_integrations(&["scryer".into()]).await.unwrap();
        store.close().await;
        let store = crate::Store::open(&db).await.unwrap();
        assert_eq!(store.enabled_integrations().await.unwrap(), ["scryer"], "the user's choice is kept");
        assert!(store.own_integrations_off().await.unwrap().is_empty(), "PDT's own are on");
        store.set_own_integrations_off(&["github".into()]).await.unwrap();
        assert_eq!(store.own_integrations_off().await.unwrap(), ["github"]);
    }

    #[tokio::test]
    async fn corrupt_host_settings_are_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        sqlx::query("INSERT INTO host_settings (key, value) VALUES ('host', 'not json')")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(matches!(store.get_host_settings().await, Err(StoreError::Corrupt(_))));
    }

    #[tokio::test]
    async fn project_settings_roundtrip_and_cascade() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        let s = ProjectSettings {
            run_overrides: Some("[[run]]\nname = \"web\"\n".into()),
            agent_credentials: Some(CredentialPolicy::PassThrough),
            agent_instructions: Some("Run `make check` before pushing.".into()),
            merge_via_pr: true,
            agent_permission_mode: Some(pdt_core::conversation::PermissionMode::AcceptEdits),
            policy: pdt_core::policy::ProjectPolicy {
                merge_by: pdt_core::policy::MergeBy::User,
                require_approval: true,
                max_running_attempts: Some(2),
                auto_start: pdt_core::policy::AutoStart::Ready,
                auto_start_order: pdt_core::policy::AutoStartOrder::Rank,
                approval_exempt_labels: vec!["automerge".into()],
                pm_acts: true,
                start_delay_seconds: 30,
                require_green_ci: false,
                triage_approval: pdt_core::policy::TriageApproval::Off,
                resolve_conflicts: pdt_core::policy::ResolveConflicts::Immediately,
                hot_areas: vec!["api".into(), "migrations".into()],
                untagged_alongside_hot: false,
            },
            integrations: [
                ("tracker", json!({"connection": "work", "scope": "project = ABC", "done": "Close"})),
                ("ci", json!({"connection": "builds", "pipelines": ["app", "e2e"]})),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.as_object().unwrap().clone()))
            .collect(),
        };
        store.put_project_settings("p", &s).await.unwrap();
        let got = store.get_project_settings("p").await.unwrap();
        assert_eq!(got, s);
        assert_eq!(got.integration::<serde_json::Value>("tracker").unwrap()["done"], "Close");
        assert_eq!(got.integration::<serde_json::Value>("ci").unwrap()["pipelines"], json!(["app", "e2e"]));
        store.put_project_settings("p", &ProjectSettings::default()).await.unwrap();
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        assert!(store.put_project_settings("missing", &s).await.is_err(), "foreign key enforced");
    }
}
