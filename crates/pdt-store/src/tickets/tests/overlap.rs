use super::*;

#[tokio::test]
async fn overlap_rows_are_open_tickets_with_areas_or_attempts_where_work_is_active() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let areas = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    add(&store, "a", ValidNewTicket { areas: areas(&["web", "db"]), ..new("A") }).await;
    add(&store, "b", ValidNewTicket { areas: areas(&["web"]), ..new("B") }).await;
    add(&store, "c", new("C")).await;
    add(&store, "d", ValidNewTicket { areas: areas(&["web"]), stage: TicketStage::Done, ..new("D") }).await;
    assert!(store.overlap_rows(None).await.unwrap().is_empty(), "nothing is active");

    store.start_attempt("at1", "c", None, "x", "main", "abc", json!({}), 5).await.unwrap();
    let rows = store.overlap_rows(Some("p")).await.unwrap();
    let got: Vec<(&str, Option<&str>, Vec<String>)> =
        rows.iter().map(|r| (r.ticket_id.as_str(), r.attempt_id.as_deref(), r.areas.clone())).collect();
    assert_eq!(got, [("a", None, areas(&["db", "web"])), ("b", None, areas(&["web"])), ("c", Some("at1"), vec![])]);
    assert_eq!(rows[2].key, "PX-3");
    assert!(store.overlap_rows(Some("other")).await.unwrap().is_empty());
}

#[tokio::test]
async fn overlap_reports_fold_each_open_tickets_events_per_other_ticket() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", new("A")).await;
    add(&store, "d", ValidNewTicket { stage: TicketStage::Done, ..new("D") }).await;
    assert!(store.overlap_reports().await.unwrap().is_empty());
    let ev = |t: &'static str, other: &str, files: &[&str], level: &str, cleared: bool, now| {
        let mut data = json!({"ticketId": other, "key": "PX-9", "files": files, "level": level});
        if cleared {
            data["cleared"] = json!(true);
        }
        let store = store.clone();
        async move { store.add_ticket_event(t, EventKind::Overlap, Actor::System, None, Some(data), now).await }
    };
    ev("a", "b", &["x"], "overlap", false, 2).await.unwrap();
    ev("a", "c", &["y"], "conflict", false, 3).await.unwrap();
    ev("a", "b", &["z"], "overlap", false, 4).await.unwrap();
    ev("a", "c", &[], "none", true, 5).await.unwrap();
    ev("d", "b", &["x"], "overlap", false, 6).await.unwrap();
    let r = store.overlap_reports().await.unwrap();
    let key = |t: &str, o: &str| (t.to_string(), o.to_string());
    assert_eq!(r.len(), 2, "done tickets are left out: {r:?}");
    let ab = &r[&key("a", "b")];
    assert_eq!(ab.files.iter().map(String::as_str).collect::<Vec<_>>(), ["x", "z"]);
    assert_eq!((ab.level, ab.cleared), (OverlapLevel::Overlap, false));
    assert_eq!(r[&key("a", "c")], Reported { cleared: true, ..Reported::default() });
}
