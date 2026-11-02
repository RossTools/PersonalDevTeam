//! Overlap detection's view of open tickets and their reported overlaps.

use super::*;

/// An open ticket for overlap detection (see `Store::overlap_rows`).
#[derive(Debug, Clone, PartialEq)]
pub struct OverlapRow {
    pub ticket_id: String,
    pub project_id: String,
    pub key: String,
    pub number: u32,
    pub title: String,
    pub stage: TicketStage,
    /// Its active attempt, if any.
    pub attempt_id: Option<String>,
    /// The active attempt's worktree.
    pub worktree_id: Option<String>,
    /// Sorted.
    pub areas: Vec<String>,
}

impl Store {
    /// The open tickets that take part in overlap detection, in projects that have an active attempt (all projects,
    /// or just `project`): those with an active attempt and those that list areas.
    pub async fn overlap_rows(&self, project: Option<&str>) -> Result<Vec<OverlapRow>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let mut qb = QueryBuilder::<Sqlite>::new(
            "SELECT t.id, t.project_id, COALESCE(p.key, '') AS pkey, t.number, t.title, t.stage, \
             (SELECT a.id FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active') AS attempt_id, \
             (SELECT a.worktree_id FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active') AS worktree_id \
             FROM tickets t JOIN projects p ON p.id = t.project_id \
             WHERE t.stage NOT IN ('done', 'wont_do') \
             AND NOT EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id) \
             AND EXISTS (SELECT 1 FROM attempts a JOIN tickets o ON o.id = a.ticket_id \
                         WHERE a.state = 'active' AND o.project_id = t.project_id) \
             AND (EXISTS (SELECT 1 FROM ticket_areas ta WHERE ta.ticket_id = t.id) \
                  OR EXISTS (SELECT 1 FROM attempts a WHERE a.ticket_id = t.id AND a.state = 'active'))",
        );
        if let Some(p) = project {
            qb.push(" AND t.project_id = ").push_bind(p.to_string());
        }
        qb.push(" ORDER BY t.project_id, t.number");
        let mut rows = qb
            .build()
            .fetch_all(&mut *conn)
            .await?
            .iter()
            .map(|r| {
                let pkey: String = r.try_get("pkey")?;
                let number = u32::try_from(r.try_get::<i64, _>("number")?).unwrap_or(0);
                Ok(OverlapRow {
                    ticket_id: r.try_get("id")?,
                    project_id: r.try_get("project_id")?,
                    key: format_key(&pkey, number),
                    number,
                    title: r.try_get("title")?,
                    stage: stage_of(&r.try_get::<String, _>("stage")?)?,
                    attempt_id: r.try_get("attempt_id")?,
                    worktree_id: r.try_get("worktree_id")?,
                    areas: vec![],
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let ids: Vec<String> = rows.iter().map(|r| r.ticket_id.clone()).collect();
        let mut areas: HashMap<String, Vec<String>> = HashMap::new();
        for chunk in ids.chunks(500) {
            let mut qb = QueryBuilder::<Sqlite>::new("SELECT ticket_id, area FROM ticket_areas WHERE ticket_id IN ");
            push_in(&mut qb, chunk);
            qb.push(" ORDER BY 1, 2");
            for r in qb.build().fetch_all(&mut *conn).await? {
                areas.entry(r.try_get(0)?).or_default().push(r.try_get(1)?);
            }
        }
        for r in rows.iter_mut() {
            r.areas = areas.remove(&r.ticket_id).unwrap_or_default();
        }
        Ok(rows)
    }

    /// What the timelines of open tickets have said about their file overlaps, by (ticket id, other ticket id): every
    /// `overlap` event folded in order ([`Reported::add`]).
    pub async fn overlap_reports(&self) -> Result<HashMap<(String, String), Reported>, StoreError> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT e.ticket_id, e.data FROM ticket_events e JOIN tickets t ON t.id = e.ticket_id \
             WHERE e.kind = 'overlap' AND t.stage NOT IN ('done', 'wont_do') ORDER BY e.created_at, e.id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out: HashMap<(String, String), Reported> = HashMap::new();
        for (ticket_id, data) in rows {
            let Some(v) = data.and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok()) else { continue };
            let Some(other) = v.get("ticketId").and_then(|t| t.as_str()) else { continue };
            let files = v.get("files").and_then(|f| f.as_array()).into_iter().flatten();
            let level = v.get("level").and_then(|l| serde_json::from_value(l.clone()).ok()).unwrap_or_default();
            let cleared = v.get("cleared").and_then(|c| c.as_bool()).unwrap_or(false);
            out.entry((ticket_id, other.to_string())).or_default().add(
                files.filter_map(|f| f.as_str().map(String::from)),
                level,
                cleared,
            );
        }
        Ok(out)
    }
}
