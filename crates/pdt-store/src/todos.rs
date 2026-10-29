//! The user's todo list (PDT-125), with the project and ticket keys its chips show. A repeating todo (PDT-141) makes
//! its next one when it is checked off (or deleted as "only this one"), once, in the same transaction.

use crate::{Store, StoreError};
use pdt_core::todo::Todo;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

const SELECT: &str = "SELECT t.id, t.project_id, t.ticket_id, t.title, t.body, t.done, t.position, t.created_at, \
                      t.updated_at, t.done_at, t.due_on, t.due_time, t.recurrence, p.key AS project_key, k.number AS ticket_number, \
                      kp.key AS ticket_project_key, k.title AS ticket_title \
                      FROM todos t \
                      LEFT JOIN projects p ON p.id = t.project_id \
                      LEFT JOIN tickets k ON k.id = t.ticket_id \
                      LEFT JOIN projects kp ON kp.id = k.project_id";

fn to_todo(r: &SqliteRow) -> Result<Todo, StoreError> {
    let number: Option<i64> = r.try_get("ticket_number")?;
    let ticket_project: Option<String> = r.try_get("ticket_project_key")?;
    Ok(Todo {
        id: r.try_get("id")?,
        project_id: r.try_get("project_id")?,
        ticket_id: r.try_get("ticket_id")?,
        title: r.try_get("title")?,
        body: r.try_get("body")?,
        done: r.try_get::<i64, _>("done")? != 0,
        position: r.try_get("position")?,
        created_at: r.try_get("created_at")?,
        updated_at: r.try_get("updated_at")?,
        done_at: r.try_get("done_at")?,
        project_key: r.try_get::<Option<String>, _>("project_key")?.filter(|k| !k.is_empty()),
        ticket_key: number.map(|n| match ticket_project.as_deref().filter(|k| !k.is_empty()) {
            Some(key) => format!("{key}-{n}"),
            None => format!("#{n}"),
        }),
        ticket_title: r.try_get("ticket_title")?,
        due_on: r.try_get("due_on")?,
        due_time: r.try_get("due_time")?,
        // A rule this build can't read is no rule (rather than a list that won't load).
        recurrence: r.try_get::<Option<String>, _>("recurrence")?.and_then(|j| serde_json::from_str(&j).ok()),
    })
}

fn recurrence_json(t: &Todo) -> Option<String> {
    t.recurrence.as_ref().map(|r| serde_json::to_string(r).expect("a recurrence serializes"))
}

/// Adds `t` (its keys are ignored: they come from the links).
async fn insert(e: impl sqlx::SqliteExecutor<'_>, t: &Todo) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO todos (id, project_id, ticket_id, title, body, done, position, created_at, updated_at, \
         done_at, due_on, due_time, recurrence) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&t.id)
    .bind(&t.project_id)
    .bind(&t.ticket_id)
    .bind(&t.title)
    .bind(&t.body)
    .bind(t.done)
    .bind(t.position)
    .bind(t.created_at)
    .bind(t.updated_at)
    .bind(t.done_at)
    .bind(&t.due_on)
    .bind(&t.due_time)
    .bind(recurrence_json(t))
    .execute(e)
    .await?;
    Ok(())
}

async fn update(e: impl sqlx::SqliteExecutor<'_>, t: &Todo) -> Result<bool, StoreError> {
    let r = sqlx::query(
        "UPDATE todos SET project_id = ?, ticket_id = ?, title = ?, body = ?, done = ?, position = ?, \
         updated_at = ?, done_at = ?, due_on = ?, due_time = ?, recurrence = ? WHERE id = ?",
    )
    .bind(&t.project_id)
    .bind(&t.ticket_id)
    .bind(&t.title)
    .bind(&t.body)
    .bind(t.done)
    .bind(t.position)
    .bind(t.updated_at)
    .bind(t.done_at)
    .bind(&t.due_on)
    .bind(&t.due_time)
    .bind(recurrence_json(t))
    .bind(&t.id)
    .execute(e)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// What a todo was before a change: open, and whether it already made its next one. None: it is gone.
async fn before(tx: &mut sqlx::SqliteConnection, id: &str) -> Result<Option<(bool, bool)>, StoreError> {
    let row: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT done, next_id FROM todos WHERE id = ?").bind(id).fetch_optional(&mut *tx).await?;
    Ok(row.map(|(done, next)| (done == 0, next.is_some())))
}

/// Adds `next` as `id`'s next todo when `id` was open and hasn't made one yet. True if it did.
async fn make_next(
    tx: &mut sqlx::SqliteConnection,
    id: &str,
    was: (bool, bool),
    next: Option<&Todo>,
) -> Result<bool, StoreError> {
    let (Some(next), (true, false)) = (next, was) else { return Ok(false) };
    insert(&mut *tx, next).await?;
    sqlx::query("UPDATE todos SET next_id = ? WHERE id = ?").bind(&next.id).bind(id).execute(&mut *tx).await?;
    Ok(true)
}

/// What `list_todos` filters on.
#[derive(Debug, Clone, Default)]
pub struct TodoFilter {
    /// Only this project's todos.
    pub project_id: Option<String>,
    pub done: Option<bool>,
}

impl Store {
    /// The todos matching the filter: open ones first in their order, then done ones, latest done first.
    pub async fn list_todos(&self, f: &TodoFilter) -> Result<Vec<Todo>, StoreError> {
        let mut qb = QueryBuilder::<Sqlite>::new(SELECT);
        qb.push(" WHERE 1 = 1");
        if let Some(p) = &f.project_id {
            qb.push(" AND t.project_id = ").push_bind(p.clone());
        }
        if let Some(done) = f.done {
            qb.push(" AND t.done = ").push_bind(done);
        }
        qb.push(" ORDER BY t.done, CASE WHEN t.done THEN -t.done_at ELSE t.position END, t.created_at");
        let rows = qb.build().fetch_all(&self.pool).await?;
        rows.iter().map(to_todo).collect()
    }

    pub async fn get_todo(&self, id: &str) -> Result<Option<Todo>, StoreError> {
        let sql = format!("{SELECT} WHERE t.id = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_optional(&self.pool).await?;
        row.as_ref().map(to_todo).transpose()
    }

    /// The lowest position in use (a new todo goes above it).
    pub async fn first_todo_position(&self) -> Result<Option<f64>, StoreError> {
        Ok(sqlx::query_scalar("SELECT MIN(position) FROM todos").fetch_one(&self.pool).await?)
    }

    /// Adds a todo (its keys are ignored: they come from the links).
    pub async fn insert_todo(&self, t: &Todo) -> Result<(), StoreError> {
        insert(&self.pool, t).await
    }

    /// Replaces a todo's fields (the caller merged the patch). False if it is gone.
    pub async fn update_todo(&self, t: &Todo) -> Result<bool, StoreError> {
        update(&self.pool, t).await
    }

    /// [`Store::update_todo`] for a repeating todo being checked off: `next` is added too if the todo was open and
    /// hasn't made one before (so two racing completes, or checking it off again, make one). None if it is gone;
    /// otherwise whether `next` was added.
    pub async fn complete_todo(&self, t: &Todo, next: &Todo) -> Result<Option<bool>, StoreError> {
        let mut tx = self.write_tx().await?;
        let Some(was) = before(&mut tx, &t.id).await? else { return Ok(None) };
        update(&mut *tx, t).await?;
        let made = make_next(&mut tx, &t.id, was, Some(next)).await?;
        tx.commit().await?;
        Ok(Some(made))
    }

    /// Open todos with a due day, each with the due key its reminder was last made for.
    pub async fn dated_open_todos(&self) -> Result<Vec<(Todo, Option<String>)>, StoreError> {
        let sql = format!(
            "{} WHERE t.done = 0 AND t.due_on IS NOT NULL ORDER BY t.position",
            SELECT.replace("SELECT t.id,", "SELECT t.reminded_key, t.id,")
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).fetch_all(&self.pool).await?;
        rows.iter().map(|r| Ok((to_todo(r)?, r.try_get("reminded_key")?))).collect()
    }

    /// Records that a todo's reminder was made for `key` (its due date then).
    pub async fn set_todo_reminded(&self, id: &str, key: &str) -> Result<(), StoreError> {
        sqlx::query("UPDATE todos SET reminded_key = ? WHERE id = ?").bind(key).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_todo(&self, id: &str) -> Result<bool, StoreError> {
        self.delete_todo_keeping(id, None).await
    }

    /// Deletes a todo; `next` (a repeating one deleted as "only this one") takes its place if it was open and hasn't
    /// made its next one already. False if it was gone.
    pub async fn delete_todo_keeping(&self, id: &str, next: Option<&Todo>) -> Result<bool, StoreError> {
        let mut tx = self.write_tx().await?;
        let Some(was) = before(&mut tx, id).await? else { return Ok(false) };
        sqlx::query("DELETE FROM todos WHERE id = ?").bind(id).execute(&mut *tx).await?;
        make_next(&mut tx, id, was, next).await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktrees::tests::store_with_project;

    fn todo(id: &str, position: f64) -> Todo {
        Todo {
            id: id.into(),
            project_id: None,
            ticket_id: None,
            title: id.to_uppercase(),
            body: String::new(),
            done: false,
            position,
            created_at: 1,
            updated_at: 1,
            done_at: None,
            project_key: None,
            ticket_key: None,
            ticket_title: None,
            due_on: None,
            due_time: None,
            recurrence: None,
        }
    }

    fn ids(v: Vec<Todo>) -> Vec<String> {
        v.into_iter().map(|t| t.id).collect()
    }

    #[tokio::test]
    async fn todos_list_in_order_with_their_keys_and_filters() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let ticket = pdt_core::ticket::ValidNewTicket {
            title: "Fix it".into(),
            description: String::new(),
            stage: pdt_core::ticket::TicketStage::Draft,
            priority: pdt_core::ticket::Priority::None,
            labels: vec![],
            areas: vec![],
            before: None,
            after: None,
            parent: None,
            blocked_by_previous: false,
            source: Default::default(),
        };
        let k = store.create_ticket("k1", "p", &ticket, 1).await.unwrap();
        let key = store.get_project("p").await.unwrap().unwrap().key;

        store.insert_todo(&todo("b", 2.0)).await.unwrap();
        store
            .insert_todo(&Todo {
                project_id: Some("p".into()),
                ticket_id: Some(k.summary.id.clone()),
                ..todo("a", 1.0)
            })
            .await
            .unwrap();
        store.insert_todo(&Todo { done: true, done_at: Some(5), ..todo("c", 0.0) }).await.unwrap();
        store.insert_todo(&Todo { done: true, done_at: Some(9), ..todo("d", 3.0) }).await.unwrap();
        assert_eq!(store.first_todo_position().await.unwrap(), Some(0.0));

        let all = store.list_todos(&TodoFilter::default()).await.unwrap();
        assert_eq!(ids(all.clone()), ["a", "b", "d", "c"], "open by position, then latest done");
        assert_eq!(all[0].project_key.as_deref(), Some(key.as_str()));
        assert_eq!(all[0].ticket_key, Some(k.summary.key.clone()));
        assert_eq!(all[0].ticket_title.as_deref(), Some("Fix it"));
        assert_eq!((all[1].project_key.as_ref(), all[1].ticket_key.as_ref()), (None, None));

        let f = |project: Option<&str>, done: Option<bool>| TodoFilter { project_id: project.map(String::from), done };
        assert_eq!(ids(store.list_todos(&f(Some("p"), None)).await.unwrap()), ["a"]);
        assert_eq!(ids(store.list_todos(&f(None, Some(true))).await.unwrap()), ["d", "c"]);
        assert_eq!(ids(store.list_todos(&f(None, Some(false))).await.unwrap()), ["a", "b"]);

        let mut b = store.get_todo("b").await.unwrap().unwrap();
        b.position = 0.5;
        b.title = "Bee".into();
        b.body = "# Notes\n\n- **bold**\n".into();
        assert!(store.update_todo(&b).await.unwrap());
        assert_eq!(ids(store.list_todos(&f(None, Some(false))).await.unwrap()), ["b", "a"]);
        let b = store.get_todo("b").await.unwrap().unwrap();
        assert_eq!((b.title.as_str(), b.body.as_str()), ("Bee", "# Notes\n\n- **bold**\n"));
        assert_eq!(store.get_todo("a").await.unwrap().unwrap().body, "", "none by default");

        assert!(matches!(store.delete_ticket("k1").await.unwrap(), crate::DeleteTicket::Deleted));
        let a = store.get_todo("a").await.unwrap().unwrap();
        assert_eq!((a.ticket_id, a.project_id.as_deref()), (None, Some("p")), "deleting the ticket keeps the todo");

        assert!(store.delete_todo("b").await.unwrap());
        assert!(store.dated_open_todos().await.unwrap().is_empty());
        assert!(!store.delete_todo("b").await.unwrap());
        assert_eq!(store.get_todo("b").await.unwrap(), None);
    }

    #[tokio::test]
    async fn due_dates_are_kept_and_reminders_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        store
            .insert_todo(&Todo { due_on: Some("2026-10-23".into()), due_time: Some("09:00".into()), ..todo("a", 1.0) })
            .await
            .unwrap();
        store.insert_todo(&todo("b", 2.0)).await.unwrap();
        store
            .insert_todo(&Todo { due_on: Some("2026-10-20".into()), done: true, done_at: Some(3), ..todo("c", 3.0) })
            .await
            .unwrap();
        let a = store.get_todo("a").await.unwrap().unwrap();
        assert_eq!((a.due_on.as_deref(), a.due_time.as_deref()), (Some("2026-10-23"), Some("09:00")));

        let dated = store.dated_open_todos().await.unwrap();
        assert_eq!(dated.iter().map(|(t, k)| (t.id.as_str(), k.clone())).collect::<Vec<_>>(), [("a", None)]);
        store.set_todo_reminded("a", "todo:a:2026-10-23T09:00").await.unwrap();
        assert_eq!(store.dated_open_todos().await.unwrap()[0].1.as_deref(), Some("todo:a:2026-10-23T09:00"));

        // Saving the todo keeps what its reminder was made for.
        let a = Todo { due_on: Some("2026-10-24".into()), due_time: None, ..a };
        assert!(store.update_todo(&a).await.unwrap());
        let (t, k) = store.dated_open_todos().await.unwrap().remove(0);
        assert_eq!(
            (t.due_on.as_deref(), t.due_time, k.as_deref()),
            (Some("2026-10-24"), None, Some("todo:a:2026-10-23T09:00"))
        );
    }

    #[tokio::test]
    async fn a_repeating_todo_makes_its_next_one_once() {
        use pdt_core::todo::{Recurrence, RepeatFrom, RepeatRule};
        let tmp = tempfile::tempdir().unwrap();
        let store = store_with_project(tmp.path()).await;
        let daily = Recurrence { rule: RepeatRule::Day { interval: 1 }, from: RepeatFrom::Due };
        let a = Todo {
            due_on: Some("2026-10-23".into()),
            recurrence: Some(daily.clone()),
            body: "Bring the forms.".into(),
            ..todo("a", 1.0)
        };
        store.insert_todo(&a).await.unwrap();
        assert_eq!(store.get_todo("a").await.unwrap().unwrap().recurrence, Some(daily.clone()), "kept as JSON");

        let done = Todo { done: true, done_at: Some(2), ..a.clone() };
        let next = |id: &str| Todo { id: id.into(), due_on: Some("2026-10-24".into()), ..a.clone() };
        // Two completes at once: one makes the next todo.
        let (b1, b2) = (next("b1"), next("b2"));
        let (x, y) = tokio::join!(store.complete_todo(&done, &b1), store.complete_todo(&done, &b2));
        let mut made = [x.unwrap(), y.unwrap()];
        made.sort();
        assert_eq!(made, [Some(false), Some(true)]);
        let open = store.list_todos(&TodoFilter { done: Some(false), ..Default::default() }).await.unwrap();
        assert_eq!(open.len(), 1, "{open:?}");
        assert_eq!(open[0].recurrence, Some(daily.clone()));
        assert_eq!(open[0].body, "Bring the forms.", "the next one keeps the body");

        // Reopened and checked off again: no second one.
        assert!(store.update_todo(&a).await.unwrap());
        assert_eq!(store.complete_todo(&done, &next("b3")).await.unwrap(), Some(false));
        assert_eq!(store.complete_todo(&todo("gone", 0.0), &next("b4")).await.unwrap(), None);

        // Deleted as "only this one": the next takes its place, once.
        let c = Todo { due_on: Some("2026-10-23".into()), recurrence: Some(daily), ..todo("c", 3.0) };
        store.insert_todo(&c).await.unwrap();
        assert!(store.delete_todo_keeping("c", Some(&Todo { id: "c2".into(), ..c.clone() })).await.unwrap());
        assert!(store.get_todo("c2").await.unwrap().is_some());
        assert!(!store.delete_todo_keeping("c", Some(&Todo { id: "c3".into(), ..c })).await.unwrap());
        assert!(store.get_todo("c3").await.unwrap().is_none());
    }
}
