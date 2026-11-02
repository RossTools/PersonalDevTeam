use super::*;

#[tokio::test]
async fn search_matches_word_prefixes_and_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", ValidNewTicket { description: "The login form crashes".into(), ..new("Fix crash") }).await;
    add(&store, "b", new("Café menu layout")).await;
    add(&store, "c", new("Logout button")).await;
    let q = |s: &str| TicketQuery { q: Some(s.into()), ..in_project() };
    assert_eq!(ids(&store, q("log")).await, ["c", "a"]);
    assert_eq!(ids(&store, q("login cra")).await, ["a"]);
    assert_eq!(ids(&store, q("cafe")).await, ["b"]);
    assert_eq!(ids(&store, q("PX-2")).await, ["b"]);
    assert_eq!(ids(&store, q("px-3")).await, ["c"]);
    assert_eq!(ids(&store, q("1")).await, ["a"]);
    assert_eq!(ids(&store, q("OTHER-1")).await, Vec::<String>::new());
    assert_eq!(ids(&store, q("\"quoted\" (stuff) *")).await, Vec::<String>::new());

    // Edits re-index; deletes un-index.
    let patch =
        ValidPatch { title: Some("Sign-in crash".into()), description: Some(String::new()), ..Default::default() };
    store.patch_ticket("a", &patch, 2000).await.unwrap();
    assert_eq!(ids(&store, q("login")).await, Vec::<String>::new());
    assert_eq!(ids(&store, q("sign")).await, ["a"]);
    store.delete_ticket("c").await.unwrap();
    assert_eq!(ids(&store, q("logout")).await, Vec::<String>::new());
}

#[tokio::test]
async fn a_searched_key_lists_its_ticket_first_on_every_page_size() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    // PX-2 mentions PX-1, and PX-10 mentions PX-10 (which the search prefix-matches).
    add(&store, "a", new("Mentions PX-1")).await;
    for i in 2..=10 {
        let t = match i {
            2 => "Also PX-1".to_string(),
            10 => "See PX-10".to_string(),
            _ => format!("Ticket {i}"),
        };
        add(&store, &format!("t{i}"), new(&t)).await;
    }
    let first = store.ticket_id_by_key("PX-1").await.unwrap().unwrap();
    assert_eq!(store.ticket_id_by_key("px-1").await.unwrap(), Some(first.clone()), "any case");
    assert_eq!(store.ticket_id_by_key("PX-99").await.unwrap(), None);
    assert_eq!(store.ticket_id_by_key("OTHER-1").await.unwrap(), None);
    assert_eq!(store.ticket_id_by_key("12").await.unwrap(), None, "a key needs its project");
    assert_eq!(first, "a");

    // Updated, newest first (ties by id, descending): PX-1 ("a") sorts last.
    let q = |limit, cursor| TicketQuery {
        q: Some("px-1".into()),
        sort: Some(TicketSort::Updated),
        limit: Some(limit),
        cursor,
        ..in_project()
    };
    let all = ids(&store, q(50, None)).await;
    assert_eq!(all, ["a", "t2", "t10"], "the key's ticket first, then the rest sorted");
    for limit in [1, 2] {
        let (mut got, mut cursor) = (vec![], None);
        loop {
            let page = store.query_tickets(&filter(q(limit, cursor))).await.unwrap();
            assert_eq!(page.total as usize, all.len());
            got.extend(page.items.into_iter().map(|t| t.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(got, all, "pages of {limit} list each ticket once, the key's first");
    }
}

#[tokio::test]
async fn filters_combine() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    add(&store, "a", ValidNewTicket { labels: l(&["bug", "ui"]), priority: Priority::High, ..new("A") }).await;
    add(&store, "b", ValidNewTicket { labels: l(&["bug"]), stage: TicketStage::Ready, ..new("B") }).await;
    add(&store, "c", ValidNewTicket { labels: l(&["ui"]), stage: TicketStage::Done, ..new("C") }).await;
    let page = store.query_tickets(&filter(in_project())).await.unwrap();
    assert_eq!(page.total, 3);
    assert_eq!(page.items.iter().find(|t| t.id == "a").unwrap().labels, ["bug", "ui"]);
    assert!(page.items.iter().find(|t| t.id == "c").unwrap().closed_at.is_some());
    let mut got = ids(&store, TicketQuery { label: Some("bug".into()), ..in_project() }).await;
    got.sort();
    assert_eq!(got, ["a", "b"]);
    assert_eq!(ids(&store, TicketQuery { label: Some("bug,ui".into()), ..in_project() }).await, ["a"]);
    assert_eq!(ids(&store, TicketQuery { priority: Some("high".into()), ..in_project() }).await, ["a"]);
    let open = ids(&store, TicketQuery { stage: Some("draft,ready".into()), ..in_project() }).await;
    assert_eq!(open.len(), 2);
    assert_eq!(store.project_labels("p").await.unwrap(), ["bug", "ui"]);

    // b blocks a: a is blocked until b closes.
    store.add_link("b", "a", LinkKind::Blocks, 1).await.unwrap();
    assert_eq!(ids(&store, TicketQuery { blocked: Some(true), ..in_project() }).await, ["a"]);
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().blocked_by, 1);
    store.patch_ticket("b", &ValidPatch { stage: Some(TicketStage::WontDo), ..Default::default() }, 5).await.unwrap();
    assert_eq!(ids(&store, TicketQuery { blocked: Some(true), ..in_project() }).await, Vec::<String>::new());
    assert_eq!(ids(&store, TicketQuery { blocked: Some(false), ..in_project() }).await.len(), 3);
    assert_eq!(events(&store, "a").await.last(), Some(&EventKind::Unblocked));
}

#[tokio::test]
async fn a_thousand_tickets_page_and_sort_correctly() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let stages = [TicketStage::Draft, TicketStage::Ready, TicketStage::InProgress, TicketStage::Done];
    let mut tx = store.pool.begin().await.unwrap();
    let keys = rank::spread(1000);
    for i in 0..1000usize {
        // Inserted directly (one transaction) to keep the test quick; the API path is covered elsewhere.
        sqlx::query(
            "INSERT INTO tickets (id, seq, project_id, number, title, stage, priority, rank, created_at, updated_at) \
             VALUES (?, ?, 'p', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(format!("t{i:04}"))
        .bind(i as i64 + 1)
        .bind(i as i64 + 1)
        .bind(format!("Ticket {i} {}", if i % 10 == 0 { "searchable" } else { "plain" }))
        .bind(stages[i % 4].as_str())
        .bind(Priority::ALL[i % 5].as_str())
        .bind(&keys[(i * 7919) % 1000])
        .bind((i % 50) as i64)
        .bind(((i * 31) % 997) as i64)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();

    let mut all = store.query_tickets(&filter(TicketQuery { limit: Some(500), ..in_project() })).await.unwrap();
    assert_eq!(all.total, 1000);
    let rest = TicketQuery { limit: Some(500), cursor: all.next_cursor.clone(), ..in_project() };
    all.items.extend(store.query_tickets(&filter(rest)).await.unwrap().items);
    assert_eq!(all.items.len(), 1000);

    for sort in TicketSort::ALL {
        let mut expected = all.items.clone();
        match sort {
            TicketSort::Rank => expected.sort_by(|a, b| (&a.rank, &a.id).cmp(&(&b.rank, &b.id))),
            TicketSort::Priority => expected.sort_by(|a, b| {
                (std::cmp::Reverse(a.priority.weight()), &a.rank, &a.id).cmp(&(
                    std::cmp::Reverse(b.priority.weight()),
                    &b.rank,
                    &b.id,
                ))
            }),
            TicketSort::Updated => expected.sort_by(|a, b| (b.updated_at, &b.id).cmp(&(a.updated_at, &a.id))),
            TicketSort::Created => expected.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id))),
        }
        for stage_filter in [None, Some("ready,done")] {
            let want: Vec<&str> = expected
                .iter()
                .filter(|t| stage_filter.is_none() || matches!(t.stage, TicketStage::Ready | TicketStage::Done))
                .map(|t| t.id.as_str())
                .collect();
            let mut got: Vec<String> = vec![];
            let mut cursor = None;
            loop {
                let q = TicketQuery {
                    sort: Some(*sort),
                    limit: Some(37),
                    cursor,
                    stage: stage_filter.map(Into::into),
                    ..in_project()
                };
                let page = store.query_tickets(&filter(q)).await.unwrap();
                assert_eq!(page.total as usize, want.len());
                got.extend(page.items.into_iter().map(|t| t.id));
                match page.next_cursor {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }
            assert_eq!(got, want, "{sort:?} {stage_filter:?}");
        }
    }
    let page = store
        .query_tickets(&filter(TicketQuery { q: Some("search".into()), limit: Some(30), ..in_project() }))
        .await
        .unwrap();
    assert_eq!((page.total, page.items.len(), page.next_cursor.is_some()), (100, 30, true));
    let bad = TicketQuery { sort: Some(TicketSort::Updated), cursor: Some("r:V:x".into()), ..in_project() };
    assert!(matches!(store.query_tickets(&filter(bad)).await, Err(TicketError::Invalid(_))));
}

#[tokio::test]
async fn closed_after_keeps_open_tickets_and_recently_closed_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "open", new("Open")).await;
    add(&store, "old", new("Old")).await;
    add(&store, "recent", new("Recent")).await;
    let done = ValidPatch { stage: Some(TicketStage::Done), ..Default::default() };
    store.patch_ticket("old", &done, 100).await.unwrap();
    store.patch_ticket("recent", &done, 200).await.unwrap();
    let mut got = ids(&store, TicketQuery { closed_after: Some(150), ..in_project() }).await;
    got.sort();
    assert_eq!(got, ["open", "recent"]);
    let mut got = ids(&store, TicketQuery { closed_after: Some(100), ..in_project() }).await;
    got.sort();
    assert_eq!(got, ["old", "open", "recent"]);
}

#[tokio::test]
async fn id_filters_narrow_the_list_and_its_total() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["a", "b", "c"] {
        add(&store, id, new(id)).await;
    }
    let mut f = filter(in_project());
    f.only_ids = Some(vec!["a".into(), "c".into()]);
    let page = store.query_tickets(&f).await.unwrap();
    assert_eq!((page.total, page.items.len()), (2, 2));
    f.only_ids = Some(vec![]);
    assert_eq!(store.query_tickets(&f).await.unwrap().total, 0);
    f.only_ids = None;
    f.except_ids = vec!["b".into()];
    let got: Vec<String> = store.query_tickets(&f).await.unwrap().items.into_iter().map(|t| t.id).collect();
    assert_eq!(got, ["c", "a"]);
}
