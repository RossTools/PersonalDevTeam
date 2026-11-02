//! Reading tickets: the backlog query (filters, full-text search, keyset paging) and lookups.

use super::*;

/// The FTS5 query for `q`: every word, prefix-matched (`"fix"* "log"*`). None when `q` has no words.
fn fts_query(q: &str) -> Option<String> {
    let words: Vec<String> =
        q.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| format!("\"{w}\"*")).collect();
    if words.is_empty() { None } else { Some(words.join(" ")) }
}

fn push_filters(qb: &mut QueryBuilder<Sqlite>, f: &TicketFilter) {
    qb.push(" WHERE 1 = 1");
    if let Some(p) = &f.project {
        qb.push(" AND t.project_id = ").push_bind(p.clone());
    }
    if !f.stages.is_empty() {
        qb.push(" AND t.stage IN ");
        push_in(qb, &f.stages.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
    }
    if !f.priorities.is_empty() {
        qb.push(" AND t.priority IN ");
        push_in(qb, &f.priorities.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>());
    }
    for l in &f.labels {
        qb.push(" AND EXISTS (SELECT 1 FROM ticket_labels tl WHERE tl.ticket_id = t.id AND tl.label = ")
            .push_bind(l.clone())
            .push(")");
    }
    if let Some(ids) = &f.only_ids {
        if ids.is_empty() {
            qb.push(" AND 0");
        } else {
            qb.push(" AND t.id IN ");
            push_in(qb, ids);
        }
    }
    if !f.except_ids.is_empty() {
        qb.push(" AND t.id NOT IN ");
        push_in(qb, &f.except_ids);
    }
    match &f.parent {
        Some(ParentFilter::Of(id)) => {
            qb.push(" AND t.parent_id = ").push_bind(id.clone());
        }
        Some(ParentFilter::None) => {
            qb.push(" AND t.parent_id IS NULL");
        }
        None => {}
    }
    match f.has_parts {
        Some(true) => {
            qb.push(" AND EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id)");
        }
        Some(false) => {
            qb.push(" AND NOT EXISTS (SELECT 1 FROM tickets c WHERE c.parent_id = t.id)");
        }
        None => {}
    }
    // Proposals wait in the Inbox, off the board and lists, until the user decides (PDT-366).
    qb.push(format!(" AND {}{IN_INBOX}", if f.proposed { "" } else { "NOT " }));
    if let Some(after) = f.closed_after {
        qb.push(" AND (t.closed_at IS NULL OR t.closed_at >= ").push_bind(after).push(")");
    }
    match f.blocked {
        Some(true) => {
            qb.push(format!(" AND {BLOCKED_BY} > 0"));
        }
        Some(false) => {
            qb.push(format!(" AND {BLOCKED_BY} = 0"));
        }
        None => {}
    }
    if let Some(q) = &f.q {
        let fts = fts_query(q);
        let key = parse_key(q);
        qb.push(" AND (0");
        if let Some(fts) = fts {
            qb.push(" OR t.seq IN (SELECT rowid FROM tickets_fts WHERE tickets_fts MATCH ").push_bind(fts).push(")");
        }
        if let Some(k) = key {
            qb.push(" OR (t.number = ").push_bind(k.number as i64);
            if let Some(pk) = k.key {
                qb.push(" AND p.key = ").push_bind(pk);
            }
            qb.push(")");
        }
        qb.push(")");
    }
}

/// The cursor after a first page that held only the ticket a searched key names: the sorted rows from their start.
const AFTER_EXACT: &str = "x";

/// Keyset cursors: the sort, then the last row's sort values and id.
fn encode_cursor(sort: TicketSort, t: &TicketSummary) -> String {
    match sort {
        TicketSort::Rank => format!("r:{}:{}", t.rank, t.id),
        TicketSort::Priority => format!("p:{}:{}:{}", t.priority.weight(), t.rank, t.id),
        TicketSort::Updated => format!("u:{}:{}", t.updated_at, t.id),
        TicketSort::Created => format!("c:{}:{}", t.created_at, t.id),
    }
}

fn push_cursor(qb: &mut QueryBuilder<Sqlite>, sort: TicketSort, cursor: &str) -> TResult<()> {
    let bad = || TicketError::Invalid("that cursor is not valid for this sort".into());
    let num = |s: &str| s.parse::<i64>().map_err(|_| bad());
    match sort {
        TicketSort::Rank => {
            let mut it = cursor.strip_prefix("r:").ok_or_else(bad)?.splitn(2, ':');
            let (r, id) = (it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?);
            qb.push(" AND (t.rank, t.id) > (").push_bind(r.to_string()).push(", ").push_bind(id.to_string()).push(")");
        }
        TicketSort::Priority => {
            let mut it = cursor.strip_prefix("p:").ok_or_else(bad)?.splitn(3, ':');
            let (w, r, id) = (num(it.next().ok_or_else(bad)?)?, it.next().ok_or_else(bad)?, it.next().ok_or_else(bad)?);
            qb.push(format!(" AND ({PRIORITY_WEIGHT} < "))
                .push_bind(w)
                .push(format!(" OR ({PRIORITY_WEIGHT} = "))
                .push_bind(w)
                .push(" AND (t.rank, t.id) > (")
                .push_bind(r.to_string())
                .push(", ")
                .push_bind(id.to_string())
                .push(")))");
        }
        TicketSort::Updated | TicketSort::Created => {
            let (prefix, col) = if sort == TicketSort::Updated { ("u:", "updated_at") } else { ("c:", "created_at") };
            let mut it = cursor.strip_prefix(prefix).ok_or_else(bad)?.splitn(2, ':');
            let (ts, id) = (num(it.next().ok_or_else(bad)?)?, it.next().ok_or_else(bad)?);
            qb.push(format!(" AND (t.{col}, t.id) < (")).push_bind(ts).push(", ").push_bind(id.to_string()).push(")");
        }
    }
    Ok(())
}

fn order_by(sort: TicketSort) -> String {
    match sort {
        TicketSort::Rank => " ORDER BY t.rank, t.id".into(),
        TicketSort::Priority => format!(" ORDER BY {PRIORITY_WEIGHT} DESC, t.rank, t.id"),
        TicketSort::Updated => " ORDER BY t.updated_at DESC, t.id DESC".into(),
        TicketSort::Created => " ORDER BY t.created_at DESC, t.id DESC".into(),
    }
}

impl Store {
    pub async fn get_ticket_summary(&self, id: &str) -> Result<Option<TicketSummary>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        summary(&mut conn, id).await
    }

    /// The full ticket: description, links (with the other ticket's key, title and stage) and attempts.
    pub async fn get_ticket(&self, id: &str) -> Result<Option<Ticket>, StoreError> {
        let mut conn = self.pool.acquire().await?;
        let Some(s) = summary(&mut conn, id).await? else { return Ok(None) };
        let (description,): (String,) =
            sqlx::query_as("SELECT description FROM tickets WHERE id = ?").bind(id).fetch_one(&mut *conn).await?;
        let rows = sqlx::query(
            "SELECT l.kind, 'outgoing' AS dir, o.id, COALESCE(p.key, '') AS pkey, o.number, o.title, o.stage \
             FROM ticket_links l JOIN tickets o ON o.id = l.to_id JOIN projects p ON p.id = o.project_id \
             WHERE l.from_id = ?1 \
             UNION ALL \
             SELECT l.kind, 'incoming', o.id, COALESCE(p.key, ''), o.number, o.title, o.stage \
             FROM ticket_links l JOIN tickets o ON o.id = l.from_id JOIN projects p ON p.id = o.project_id \
             WHERE l.to_id = ?1 \
             ORDER BY 1, 2, 5",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?;
        let links = rows
            .iter()
            .map(|r| {
                let kind: String = r.try_get(0)?;
                let dir: String = r.try_get(1)?;
                let pkey: String = r.try_get(3)?;
                Ok(LinkedTicket {
                    kind: LinkKind::parse(&kind).ok_or_else(|| corrupt("link kind", &kind))?,
                    direction: if dir == "outgoing" { LinkDirection::Outgoing } else { LinkDirection::Incoming },
                    id: r.try_get(2)?,
                    key: format_key(&pkey, u32::try_from(r.try_get::<i64, _>(4)?).unwrap_or(0)),
                    title: r.try_get(5)?,
                    stage: stage_of(&r.try_get::<String, _>(6)?)?,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let attempts = sqlx::query(
            "SELECT id, ticket_id, branch, base_branch, worktree_id, state, created_at, ended_at, last_head, workspace_id FROM attempts \
             WHERE ticket_id = ? ORDER BY created_at DESC, id DESC",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(to_attempt)
        .collect::<Result<Vec<_>, _>>()?;
        let sessions = sqlx::query(
            "SELECT s.id, s.title, s.status, s.agent, s.attempt_id, s.mode, pe.name AS persona FROM sessions s \
             LEFT JOIN attempts a ON a.id = s.attempt_id LEFT JOIN personas pe ON pe.id = s.persona_id WHERE COALESCE(a.ticket_id, s.ticket_id) = ? \
             ORDER BY s.created_at DESC, s.id DESC",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|r| {
            let status: String = r.try_get("status")?;
            let agent: Option<String> = r.try_get("agent")?;
            let mode: String = r.try_get("mode")?;
            Ok(AttemptSession {
                id: r.try_get("id")?,
                title: r.try_get("title")?,
                status: SessionStatus::parse(&status).ok_or_else(|| corrupt("session status", &status))?,
                agent: agent.map(|a| AgentKind::parse_stored(&a).ok_or_else(|| corrupt("session agent", &a))).transpose()?,
                attempt_id: r.try_get("attempt_id")?,
                persona: r.try_get("persona")?,
                mode: SessionMode::parse(&mode).ok_or_else(|| corrupt("session mode", &mode))?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
        let pkey = s.key.rsplit_once('-').map(|(k, _)| k).unwrap_or("");
        let default_branch = attempt_branch(pkey, s.number, &s.title);
        let sql = format!("{} WHERE t.parent_id = ? ORDER BY t.rank, t.id", summary_select());
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(&mut *conn).await?;
        let mut part_list = rows.iter().map(to_summary).collect::<Result<Vec<_>, _>>()?;
        decorate(&mut conn, &mut part_list).await?;
        let web_links = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, url, label FROM ticket_web_links WHERE ticket_id = ? ORDER BY created_at, id",
        )
        .bind(id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|(id, url, label)| WebLink { id, url, label })
        .collect();
        Ok(Some(Ticket {
            summary: s,
            description,
            links,
            attempts,
            default_branch,
            sessions,
            overlaps: vec![],
            part_list,
            web_links,
        }))
    }

    /// The backlog: filtered, searched, sorted and paged with a keyset cursor, plus the total that match.
    pub async fn query_tickets(&self, f: &TicketFilter) -> TResult<TicketPage> {
        let mut conn = self.pool.acquire().await?;
        let mut count = QueryBuilder::<Sqlite>::new(format!("SELECT COUNT(*){FROM}"));
        push_filters(&mut count, f);
        let (total,): (i64,) = count.build_query_as().fetch_one(&mut *conn).await?;

        // A search for a key lists its ticket first (PDT-271): it leaves the sorted rows, and leads the first page.
        let exact = match f.q.as_deref().and_then(parse_key) {
            Some(KeyRef { key: Some(pk), number }) => {
                let mut qb = QueryBuilder::<Sqlite>::new(summary_select());
                push_filters(&mut qb, f);
                qb.push(" AND p.key = ").push_bind(pk).push(" AND t.number = ").push_bind(i64::from(number));
                qb.build().fetch_optional(&mut *conn).await?.as_ref().map(to_summary).transpose()?
            }
            _ => None,
        };
        let first = exact.as_ref().filter(|_| f.cursor.is_none());
        let limit = f.limit as usize - usize::from(first.is_some());

        let mut qb = QueryBuilder::<Sqlite>::new(summary_select());
        push_filters(&mut qb, f);
        if let Some(e) = &exact {
            qb.push(" AND t.id != ").push_bind(e.id.clone());
        }
        if let Some(c) = f.cursor.as_deref().filter(|c| *c != AFTER_EXACT) {
            push_cursor(&mut qb, f.sort, c)?;
        }
        qb.push(order_by(f.sort));
        qb.push(" LIMIT ").push_bind(limit as i64 + 1);
        let rows = qb.build().fetch_all(&mut *conn).await?;
        let mut items = rows.iter().map(to_summary).collect::<Result<Vec<_>, _>>()?;
        let more = items.len() > limit;
        items.truncate(limit);
        let next_cursor = match items.last() {
            _ if !more => None,
            Some(t) => Some(encode_cursor(f.sort, t)),
            // A one-row first page holds only the key's ticket: the next starts the sorted rows.
            None => Some(AFTER_EXACT.to_string()),
        };
        if let Some(e) = first {
            items.insert(0, e.clone());
        }
        decorate(&mut conn, &mut items).await?;
        Ok(TicketPage { items, next_cursor, total: u32::try_from(total).unwrap_or(u32::MAX) })
    }

    /// The id of the ticket a key names (`PDT-12`, any case). None for an unknown key or anything not a key.
    pub async fn ticket_id_by_key(&self, key: &str) -> Result<Option<String>, StoreError> {
        let Some(KeyRef { key: Some(pk), number }) = parse_key(key) else { return Ok(None) };
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT t.id FROM tickets t JOIN projects p ON p.id = t.project_id WHERE p.key = ? AND t.number = ?",
        )
        .bind(pk)
        .bind(i64::from(number))
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    /// The id of a project's ticket by number.
    pub async fn ticket_id_by_number(&self, project_id: &str, number: u32) -> Result<Option<String>, StoreError> {
        let row: Option<(String,)> = sqlx::query_as("SELECT id FROM tickets WHERE project_id = ? AND number = ?")
            .bind(project_id)
            .bind(i64::from(number))
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|(id,)| id))
    }

    /// Every label used in a project, sorted.
    pub async fn project_labels(&self, project_id: &str) -> Result<Vec<String>, StoreError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT l.label FROM ticket_labels l JOIN tickets t ON t.id = l.ticket_id \
             WHERE t.project_id = ? ORDER BY l.label",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(l,)| l).collect())
    }
}
