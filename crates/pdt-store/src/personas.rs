//! Personas and their versions. Every save is a new version; a persona row points at its current
//! one.

use crate::{Store, StoreError};
use pdt_core::conversation::PermissionMode;
use pdt_core::persona::{Persona, PersonaAuthor, PersonaSetup, PersonaVersion};
use pdt_core::session::AgentKind;
use sqlx::Row;
use sqlx::sqlite::SqliteRow;

const SELECT: &str = "SELECT p.id, p.name, p.role, p.description, p.builtin, p.archived, p.version, p.created_at, \
                      p.updated_at, v.agent, v.model, v.instructions, v.permission_mode, v.tools \
                      FROM personas p JOIN persona_versions v ON v.persona_id = p.id AND v.version = p.version";

const VERSION_COLUMNS: &str =
    "persona_id, version, agent, model, instructions, permission_mode, tools, author, note, created_at";

fn corrupt(what: &str, v: &str) -> StoreError {
    StoreError::Corrupt(format!("unknown persona {what} {v:?}"))
}

fn to_setup(r: &SqliteRow) -> Result<PersonaSetup, StoreError> {
    let agent: String = r.try_get("agent")?;
    let mode: Option<String> = r.try_get("permission_mode")?;
    let tools: String = r.try_get("tools")?;
    Ok(PersonaSetup {
        agent: AgentKind::parse_stored(&agent).ok_or_else(|| corrupt("agent", &agent))?,
        model: r.try_get("model")?,
        instructions: r.try_get("instructions")?,
        permission_mode: match mode {
            Some(m) => Some(PermissionMode::parse(&m).ok_or_else(|| corrupt("permission mode", &m))?),
            None => None,
        },
        tools: serde_json::from_str(&tools).map_err(|e| StoreError::Corrupt(format!("persona tools: {e}")))?,
    })
}

fn to_persona(r: &SqliteRow) -> Result<Persona, StoreError> {
    Ok(Persona {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        role: r.try_get("role")?,
        description: r.try_get("description")?,
        builtin: r.try_get("builtin")?,
        archived: r.try_get::<i64, _>("archived")? != 0,
        version: u32::try_from(r.try_get::<i64, _>("version")?).unwrap_or(0),
        setup: to_setup(r)?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
    })
}

fn to_version(r: &SqliteRow) -> Result<PersonaVersion, StoreError> {
    let author: String = r.try_get("author")?;
    Ok(PersonaVersion {
        persona_id: r.try_get("persona_id")?,
        version: u32::try_from(r.try_get::<i64, _>("version")?).unwrap_or(0),
        setup: to_setup(r)?,
        author: PersonaAuthor::parse(&author).ok_or_else(|| corrupt("author", &author))?,
        note: r.try_get("note")?,
        created_at: r.try_get("created_at")?,
    })
}

async fn insert_version(
    conn: &mut sqlx::SqliteConnection,
    id: &str,
    version: u32,
    s: &PersonaSetup,
    author: PersonaAuthor,
    note: &str,
    now: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO persona_versions (persona_id, version, agent, model, instructions, permission_mode, tools, \
         author, note, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(i64::from(version))
    .bind(s.agent.as_str())
    .bind(&s.model)
    .bind(&s.instructions)
    .bind(s.permission_mode.map(PermissionMode::as_str))
    .bind(serde_json::to_string(&s.tools).unwrap_or_else(|_| "[]".into()))
    .bind(author.as_str())
    .bind(note)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

impl Store {
    /// Adds a persona with its first version (`p.version` is ignored: it is 1). Err `Conflict` when the name is taken.
    pub async fn insert_persona(&self, p: &Persona, author: PersonaAuthor, note: &str) -> Result<(), StoreError> {
        let mut tx = self.write_tx().await?;
        let r = sqlx::query(
            "INSERT INTO personas (id, name, role, description, builtin, archived, version, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.role)
        .bind(&p.description)
        .bind(&p.builtin)
        .bind(p.archived)
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(&mut *tx)
        .await;
        if let Err(sqlx::Error::Database(e)) = &r
            && e.is_unique_violation()
        {
            return Err(StoreError::Conflict(format!("a persona is already called {}", p.name)));
        }
        r?;
        insert_version(&mut tx, &p.id, 1, &p.setup, author, note, p.created_at).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Saves a new version and makes it current. None for an unknown persona.
    pub async fn save_persona_version(
        &self,
        id: &str,
        s: &PersonaSetup,
        author: PersonaAuthor,
        note: &str,
        now: i64,
    ) -> Result<Option<u32>, StoreError> {
        let mut tx = self.write_tx().await?;
        let Some((version,)): Option<(i64,)> =
            sqlx::query_as("SELECT version FROM personas WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?
        else {
            return Ok(None);
        };
        let next = u32::try_from(version + 1).unwrap_or(u32::MAX);
        insert_version(&mut tx, id, next, s, author, note, now).await?;
        sqlx::query("UPDATE personas SET version = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(next))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(next))
    }

    /// Changes a persona's name, role, description or archived flag. False for an unknown persona.
    pub async fn update_persona(&self, p: &Persona, now: i64) -> Result<bool, StoreError> {
        let r = sqlx::query(
            "UPDATE personas SET name = ?, role = ?, description = ?, archived = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&p.name)
        .bind(&p.role)
        .bind(&p.description)
        .bind(p.archived)
        .bind(now)
        .bind(&p.id)
        .execute(&self.pool)
        .await;
        match r {
            Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
                Err(StoreError::Conflict(format!("a persona is already called {}", p.name)))
            }
            r => Ok(r?.rows_affected() > 0),
        }
    }

    pub async fn get_persona(&self, id: &str) -> Result<Option<Persona>, StoreError> {
        let sql = format!("{SELECT} WHERE p.id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_persona).transpose()
    }

    /// By id, else by name (any case).
    pub async fn find_persona(&self, id_or_name: &str) -> Result<Option<Persona>, StoreError> {
        if let Some(p) = self.get_persona(id_or_name).await? {
            return Ok(Some(p));
        }
        let sql = format!("{SELECT} WHERE p.name = ? COLLATE NOCASE");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id_or_name.trim()).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_persona).transpose()
    }

    pub async fn persona_by_builtin(&self, template: &str) -> Result<Option<Persona>, StoreError> {
        let sql = format!("{SELECT} WHERE p.builtin = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(template).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_persona).transpose()
    }

    /// Every persona, by name.
    pub async fn list_personas(&self) -> Result<Vec<Persona>, StoreError> {
        let sql = format!("{SELECT} ORDER BY p.name COLLATE NOCASE");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(to_persona).collect()
    }

    pub async fn delete_persona(&self, id: &str) -> Result<bool, StoreError> {
        let r = sqlx::query("DELETE FROM personas WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() > 0)
    }

    /// The built-in template PDT last applied to a persona (PDT-100); None when not recorded.
    pub async fn persona_template(&self, id: &str) -> Result<Option<PersonaSetup>, StoreError> {
        let t: Option<(Option<String>,)> = sqlx::query_as("SELECT builtin_template FROM personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        match t.and_then(|(t,)| t) {
            Some(t) => {
                serde_json::from_str(&t).map(Some).map_err(|e| StoreError::Corrupt(format!("persona template: {e}")))
            }
            None => Ok(None),
        }
    }

    pub async fn set_persona_template(&self, id: &str, template: &PersonaSetup) -> Result<(), StoreError> {
        sqlx::query("UPDATE personas SET builtin_template = ? WHERE id = ?")
            .bind(serde_json::to_string(template).unwrap_or_default())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// A persona's versions, newest first.
    pub async fn list_persona_versions(&self, id: &str) -> Result<Vec<PersonaVersion>, StoreError> {
        let sql = format!("SELECT {VERSION_COLUMNS} FROM persona_versions WHERE persona_id = ? ORDER BY version DESC");
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(&self.pool).await?;
        rows.iter().map(to_version).collect()
    }

    pub async fn get_persona_version(&self, id: &str, version: u32) -> Result<Option<PersonaVersion>, StoreError> {
        let sql = format!("SELECT {VERSION_COLUMNS} FROM persona_versions WHERE persona_id = ? AND version = ?");
        let row =
            sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).bind(i64::from(version)).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_version).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;

    fn persona(id: &str, name: &str) -> Persona {
        Persona {
            id: id.into(),
            name: name.into(),
            role: "reviewer".into(),
            description: String::new(),
            builtin: None,
            archived: false,
            version: 1,
            setup: PersonaSetup {
                agent: AgentKind::Claude,
                model: None,
                instructions: "Review.".into(),
                permission_mode: Some(PermissionMode::Plan),
                tools: vec!["get_ticket".into()],
            },
            created_at: 10,
            updated_at: 10,
        }
    }

    #[tokio::test]
    async fn versions_accumulate_and_names_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let p = persona("p1", "Reviewer");
        store.insert_persona(&p, PersonaAuthor::User, "new").await.unwrap();
        let dup = store.insert_persona(&persona("p2", "reviewer"), PersonaAuthor::User, "").await;
        assert!(matches!(dup, Err(StoreError::Conflict(_))), "{dup:?}");
        assert_eq!(store.get_persona("p1").await.unwrap().unwrap(), p);
        assert_eq!(store.find_persona("REVIEWER").await.unwrap().unwrap().id, "p1");

        let s2 = PersonaSetup { instructions: "Review hard.".into(), ..p.setup.clone() };
        assert_eq!(store.save_persona_version("p1", &s2, PersonaAuthor::User, "harder", 20).await.unwrap(), Some(2));
        assert_eq!(store.save_persona_version("nope", &s2, PersonaAuthor::User, "", 20).await.unwrap(), None);
        let now = store.get_persona("p1").await.unwrap().unwrap();
        assert_eq!((now.version, now.setup.instructions.as_str(), now.updated_at), (2, "Review hard.", 20));
        let versions = store.list_persona_versions("p1").await.unwrap();
        assert_eq!(versions.iter().map(|v| v.version).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(versions[1].setup, p.setup);
        assert_eq!(store.get_persona_version("p1", 1).await.unwrap().unwrap().note, "new");

        assert_eq!(store.persona_template("p1").await.unwrap(), None);
        store.set_persona_template("p1", &s2).await.unwrap();
        assert_eq!(store.persona_template("p1").await.unwrap(), Some(s2.clone()));

        let renamed = Persona { name: "Strict reviewer".into(), archived: true, ..now };
        assert!(store.update_persona(&renamed, 30).await.unwrap());
        let got = store.list_personas().await.unwrap();
        assert_eq!((got[0].name.as_str(), got[0].archived), ("Strict reviewer", true));
        assert!(store.delete_persona("p1").await.unwrap());
        assert!(store.list_persona_versions("p1").await.unwrap().is_empty(), "versions go with it");
    }

    /// PDT-414: a list saved empty meant the default set; the migration that made empty mean none gave each one the
    /// default set, so those personas kept their tools.
    #[tokio::test]
    async fn the_empty_tools_migration_gives_saved_empty_lists_the_default_set() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_with_project(dir.path()).await;
        let empty = PersonaSetup { tools: vec![], ..persona("p1", "Coder").setup };
        store
            .insert_persona(&Persona { setup: empty.clone(), ..persona("p1", "Coder") }, PersonaAuthor::User, "")
            .await
            .unwrap();
        store.set_persona_template("p1", &empty).await.unwrap();
        store.insert_persona(&persona("p2", "Reviewer"), PersonaAuthor::User, "").await.unwrap();
        sqlx::raw_sql(include_str!("../migrations/20261102173456_persona_empty_tools.sql"))
            .execute(&store.pool)
            .await
            .unwrap();
        let defaults: Vec<String> = pdt_core::persona::DEFAULT_TOOLS.map(String::from).to_vec();
        assert_eq!(store.get_persona("p1").await.unwrap().unwrap().setup.tools, defaults);
        assert_eq!(store.persona_template("p1").await.unwrap().unwrap().tools, defaults);
        assert_eq!(
            store.get_persona("p2").await.unwrap().unwrap().setup.tools,
            ["get_ticket"],
            "a list kept as it was"
        );
    }
}
