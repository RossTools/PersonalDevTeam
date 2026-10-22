use crate::{Store, StoreError};
use pdt_core::host::{CredentialPolicy, HostSettings};
use pdt_core::policy::{AutoStart, MergeBy, ProjectPolicy};

const HOST_KEY: &str = "host";

/// `project_settings` columns: run overrides, credential policy, agent instructions, merge through PRs, permission
/// mode, and the policies (merge by, require approval, running-attempt cap, auto-start, approval-exempt labels, the PM
/// may act).
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
}

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

    pub async fn get_project_settings(&self, project_id: &str) -> Result<ProjectSettings, StoreError> {
        let row: Option<ProjectSettingsRow> = sqlx::query_as(
            "SELECT run_overrides, agent_credentials, agent_instructions, merge_via_pr, agent_permission_mode, \
             merge_by, require_approval, max_running_attempts, auto_start, approval_exempt_labels, pm_acts \
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
        )) = row
        else {
            return Ok(ProjectSettings::default());
        };
        let agent_credentials = match creds {
            Some(c) => Some(
                CredentialPolicy::parse(&c).ok_or_else(|| StoreError::Corrupt(format!("credential policy {c:?}")))?,
            ),
            None => None,
        };
        let agent_permission_mode = mode.as_deref().and_then(pdt_core::conversation::PermissionMode::parse);
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
                approval_exempt_labels: serde_json::from_str(&exempt)
                    .map_err(|_| StoreError::Corrupt(format!("approval-exempt labels {exempt:?}")))?,
                pm_acts,
            },
        })
    }

    pub async fn put_project_settings(&self, project_id: &str, s: &ProjectSettings) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO project_settings (project_id, run_overrides, agent_credentials, agent_instructions, \
             merge_via_pr, agent_permission_mode, merge_by, require_approval, max_running_attempts, auto_start, \
             approval_exempt_labels, pm_acts) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(project_id) DO UPDATE SET \
             run_overrides = excluded.run_overrides, agent_credentials = excluded.agent_credentials, \
             agent_instructions = excluded.agent_instructions, merge_via_pr = excluded.merge_via_pr, \
             agent_permission_mode = excluded.agent_permission_mode, merge_by = excluded.merge_by, \
             require_approval = excluded.require_approval, max_running_attempts = excluded.max_running_attempts, \
             auto_start = excluded.auto_start, approval_exempt_labels = excluded.approval_exempt_labels, \
             pm_acts = excluded.pm_acts",
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
        store.delete_host_value("sensor.x.y").await.unwrap();
        assert_eq!(store.get_host_value("sensor.x.y").await.unwrap(), None);
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
                approval_exempt_labels: vec!["automerge".into()],
                pm_acts: true,
            },
        };
        store.put_project_settings("p", &s).await.unwrap();
        assert_eq!(store.get_project_settings("p").await.unwrap(), s);
        store.put_project_settings("p", &ProjectSettings::default()).await.unwrap();
        assert_eq!(store.get_project_settings("p").await.unwrap(), ProjectSettings::default());
        assert!(store.put_project_settings("missing", &s).await.is_err(), "foreign key enforced");
    }
}
