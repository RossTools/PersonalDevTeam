//! Secrets' names, access and uses (PDT-158). Never their values: those are in the OS keychain.

use crate::{Store, StoreError};
use pdt_core::secret::{Secret, SecretUse};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const SELECT: &str = "SELECT id, name, description, projects, personas, created_at, updated_at, last_used_at, \
                      last_used_by FROM secrets";

fn list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn to_secret(r: &SqliteRow) -> Result<Secret, StoreError> {
    Ok(Secret {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        description: r.try_get("description")?,
        projects: list(&r.try_get::<String, _>("projects")?),
        personas: list(&r.try_get::<String, _>("personas")?),
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        last_used_at: r.try_get("last_used_at")?,
        last_used_by: r.try_get("last_used_by")?,
    })
}

fn json(l: &[String]) -> String {
    serde_json::to_string(l).expect("a list serializes")
}

fn conflict(e: sqlx::Error, name: &str) -> StoreError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => {
            StoreError::Conflict(format!("a secret is already called {name}"))
        }
        _ => e.into(),
    }
}

impl Store {
    pub async fn insert_secret(&self, s: &Secret) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO secrets (id, name, description, projects, personas, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&s.id)
        .bind(&s.name)
        .bind(&s.description)
        .bind(json(&s.projects))
        .bind(json(&s.personas))
        .bind(s.created_at)
        .bind(s.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| conflict(e, &s.name))?;
        Ok(())
    }

    /// Writes its description, access and `updated_at`.
    pub async fn update_secret(&self, s: &Secret) -> Result<bool, StoreError> {
        let r =
            sqlx::query("UPDATE secrets SET description = ?, projects = ?, personas = ?, updated_at = ? WHERE id = ?")
                .bind(&s.description)
                .bind(json(&s.projects))
                .bind(json(&s.personas))
                .bind(s.updated_at)
                .bind(&s.id)
                .execute(&self.pool)
                .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn get_secret(&self, id: &str) -> Result<Option<Secret>, StoreError> {
        let r = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE id = ?")))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        r.as_ref().map(to_secret).transpose()
    }

    /// By name, ignoring case.
    pub async fn secret_by_name(&self, name: &str) -> Result<Option<Secret>, StoreError> {
        let r = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE name = ?")))
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        r.as_ref().map(to_secret).transpose()
    }

    /// Every secret, by name.
    pub async fn list_secrets(&self) -> Result<Vec<Secret>, StoreError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} ORDER BY name COLLATE NOCASE")))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(to_secret).collect()
    }

    /// Deletes it and its uses.
    pub async fn delete_secret(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM secrets WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// Records a use (and makes it the secret's last).
    pub async fn record_secret_use(&self, u: &SecretUse) -> Result<i64, StoreError> {
        let mut tx = self.pool.begin().await?;
        let id = sqlx::query(
            "INSERT INTO secret_uses (secret_id, at, by, how, session_id, run_id) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&u.secret_id)
        .bind(u.at)
        .bind(&u.by)
        .bind(&u.how)
        .bind(&u.session_id)
        .bind(&u.run_id)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        sqlx::query("UPDATE secrets SET last_used_at = ?, last_used_by = ? WHERE id = ?")
            .bind(u.at)
            .bind(&u.by)
            .bind(&u.secret_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// A secret's uses, newest first, at most `limit`.
    pub async fn secret_uses(&self, secret_id: &str, limit: i64) -> Result<Vec<SecretUse>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, secret_id, at, by, how, session_id, run_id FROM secret_uses WHERE secret_id = ? \
             ORDER BY at DESC, id DESC LIMIT ?",
        )
        .bind(secret_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SecretUse {
                    id: r.try_get("id")?,
                    secret_id: r.try_get("secret_id")?,
                    at: r.try_get("at")?,
                    by: r.try_get("by")?,
                    how: r.try_get("how")?,
                    session_id: r.try_get("session_id")?,
                    run_id: r.try_get("run_id")?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;

    fn secret(id: &str, name: &str) -> Secret {
        Secret {
            id: id.into(),
            name: name.into(),
            description: "d".into(),
            projects: vec!["p".into()],
            personas: vec![],
            created_at: 1,
            updated_at: 1,
            last_used_at: None,
            last_used_by: None,
        }
    }

    #[tokio::test]
    async fn secrets_and_their_uses() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.insert_secret(&secret("a", "TOKEN")).await.unwrap();
        assert!(matches!(store.insert_secret(&secret("b", "token")).await, Err(StoreError::Conflict(_))));
        assert_eq!(store.secret_by_name("token").await.unwrap().unwrap().id, "a");
        let mut s = store.get_secret("a").await.unwrap().unwrap();
        assert_eq!(s.projects, vec!["p"]);
        s.personas = vec!["coder".into()];
        s.updated_at = 2;
        assert!(store.update_secret(&s).await.unwrap());
        let u = SecretUse {
            id: 0,
            secret_id: "a".into(),
            at: 5,
            by: "Coder · PDT-1".into(),
            how: "run_with_secrets".into(),
            session_id: Some("s".into()),
            run_id: None,
        };
        store.record_secret_use(&u).await.unwrap();
        store.record_secret_use(&SecretUse { at: 6, how: "write_secret_file".into(), ..u.clone() }).await.unwrap();
        let got = store.get_secret("a").await.unwrap().unwrap();
        assert_eq!(
            (got.personas, got.last_used_at, got.last_used_by.as_deref()),
            (vec!["coder".to_string()], Some(6), Some("Coder · PDT-1"))
        );
        let uses = store.secret_uses("a", 10).await.unwrap();
        assert_eq!(uses.iter().map(|u| u.how.as_str()).collect::<Vec<_>>(), ["write_secret_file", "run_with_secrets"]);
        assert!(store.delete_secret("a").await.unwrap());
        assert!(store.secret_uses("a", 10).await.unwrap().is_empty());
        assert!(store.list_secrets().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn known_values_are_redacted_from_what_is_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store.redactor().set("TOKEN", "hunter2-value");
        let e = pdt_core::workflow::api::NewLogEvent::new(
            "webhook.received",
            "k1",
            "hooks",
            serde_json::json!({"auth": "Bearer hunter2-value"}),
        );
        let got = store.emit_event(&e, 1).await.unwrap().unwrap();
        assert_eq!(got.payload["auth"], "Bearer [secret:TOKEN]");
    }
}
