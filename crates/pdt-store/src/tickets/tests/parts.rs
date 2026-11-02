use super::*;

#[tokio::test]
async fn parts_follow_their_parent_in_order_and_can_block_each_other() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "other", new("Other")).await;
    add(&store, "par", new("Parent")).await;
    add(&store, "top", new("Top")).await;
    let a = add(&store, "a", part("5a", "par")).await;
    assert_eq!(a.summary.parent.as_ref().map(|p| p.key.as_str()), Some("PX-2"));
    let b = store
        .create_ticket("b", "p", &ValidNewTicket { blocked_by_previous: true, ..part("5b", "par") }, 2)
        .await
        .unwrap();
    assert_eq!(b.summary.blocked_by, 1, "5a blocks 5b");
    // Parts sit right after the parent, in the order made.
    assert_eq!(ids(&store, in_project()).await, ["top", "par", "a", "b", "other"]);
    let parent = store.get_ticket("par").await.unwrap().unwrap();
    assert_eq!(parent.part_list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert!(events(&store, "par").await.contains(&EventKind::Parted));
    assert!(events(&store, "a").await.contains(&EventKind::Parted));
    // Filters.
    let only = |q: TicketQuery| ids(&store, TicketQuery { project: Some("p".into()), ..q });
    assert_eq!(only(TicketQuery { parent: Some("par".into()), ..Default::default() }).await, ["a", "b"]);
    assert_eq!(only(TicketQuery { parent: Some("none".into()), ..Default::default() }).await, ["top", "par", "other"]);
    assert_eq!(only(TicketQuery { has_parts: Some(true), ..Default::default() }).await, ["par"]);
    assert_eq!(only(TicketQuery { has_parts: Some(false), ..Default::default() }).await.len(), 4);
    // Several at once, after the existing ones, each blocked by the one before.
    let v =
        ValidParts { parts: vec![("5c".into(), "c".into()), ("5d".into(), String::new())], blocked_by_previous: true };
    let made = store.create_parts("par", &v, 3).await.unwrap();
    assert_eq!(made.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["5c", "5d"]);
    assert!(made.iter().all(|t| t.blocked_by == 1));
    let rows = store.part_rows(&["par".into()]).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["5a", "5b", "5c", "5d"]);
    assert_eq!(rows.iter().map(|r| r.open_blockers).collect::<Vec<_>>(), [0, 1, 1, 1]);
    assert!(rows.iter().all(|r| r.attempt.is_none() && r.parent_id == "par"));
}

#[tokio::test]
async fn the_parent_rules_hold_in_the_store() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "par", new("Parent")).await;
    add(&store, "a", part("A", "par")).await;
    add(&store, "x", new("X")).await;
    let err = |r: TResult<Option<Ticket>>| match r {
        Err(TicketError::InvalidParent(rule)) => rule,
        other => panic!("{other:?}"),
    };
    use pdt_core::parts::ParentRule;
    let to = |p: &str| ValidPatch { parent: Some(Some(p.into())), ..Default::default() };
    assert_eq!(err(store.patch_ticket("x", &to("a"), 2).await), ParentRule::ParentIsAPart);
    assert_eq!(err(store.patch_ticket("par", &to("x"), 2).await), ParentRule::ChildHasParts);
    assert_eq!(err(store.patch_ticket("x", &to("x"), 2).await), ParentRule::Itself);
    assert!(matches!(
        store.create_ticket("y", "p", &part("Y", "a"), 2).await,
        Err(TicketError::InvalidParent(ParentRule::ParentIsAPart))
    ));
    store.start_attempt("at", "x", None, "b", "main", "c", json!({}), 3).await.unwrap();
    assert!(matches!(
        store.create_ticket("y", "p", &part("Y", "x"), 2).await,
        Err(TicketError::InvalidParent(ParentRule::ParentActive))
    ));
    // Move a ticket in, then detach it.
    add(&store, "z", new("Z")).await;
    let z = store.patch_ticket("z", &to("par"), 4).await.unwrap().unwrap();
    assert_eq!(z.summary.parent.map(|p| p.id), Some("par".into()));
    let z =
        store.patch_ticket("z", &ValidPatch { parent: Some(None), ..Default::default() }, 5).await.unwrap().unwrap();
    assert_eq!(z.summary.parent, None);
    assert_eq!(events(&store, "z").await.iter().filter(|e| **e == EventKind::Parted).count(), 2);
}

#[tokio::test]
async fn a_parent_closes_only_after_its_parts_and_is_not_deleted_with_them() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "par", new("Parent")).await;
    add(&store, "a", part("A", "par")).await;
    add(&store, "b", part("B", "par")).await;
    let done = ValidPatch { stage: Some(TicketStage::Done), ..Default::default() };
    match store.patch_ticket("par", &done, 2).await {
        Err(TicketError::PartsOpen { key, open }) => {
            assert_eq!((key.as_str(), open), ("PX-1", vec!["PX-2".to_string(), "PX-3".to_string()]))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store.delete_ticket("par").await.unwrap(), DeleteTicket::HasParts);
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(matches!(
        store.bulk_tickets(&ids(&["par", "a"]), &ValidBulk::Stage(TicketStage::Done), 3).await,
        Err(TicketError::PartsOpen { .. })
    ));
    assert!(matches!(
        store.bulk_tickets(&ids(&["par", "a"]), &ValidBulk::Delete, 3).await,
        Err(TicketError::HasParts(_))
    ));
    // Closing the parent with all its parts in one bulk move works; so does a move once they are closed.
    store.bulk_tickets(&ids(&["a"]), &ValidBulk::Stage(TicketStage::WontDo), 4).await.unwrap();
    store.bulk_tickets(&ids(&["par", "b"]), &ValidBulk::Stage(TicketStage::Done), 5).await.unwrap();
    assert_eq!(store.get_ticket("par").await.unwrap().unwrap().summary.stage, TicketStage::Done);
    // Deleting a parent with all its parts at once is fine, and removing the project still works with parts.
    store.bulk_tickets(&ids(&["par", "a", "b"]), &ValidBulk::Delete, 6).await.unwrap();
    add(&store, "p2", new("P2")).await;
    add(&store, "c", part("C", "p2")).await;
    assert!(store.delete_project("p").await.unwrap());
}

#[tokio::test]
async fn a_part_starting_moves_its_parent_once() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "par", ValidNewTicket { stage: TicketStage::Todo, ..new("Parent") }).await;
    add(&store, "a", part("A", "par")).await;
    assert!(store.part_started("par", 2).await.unwrap());
    assert!(!store.part_started("par", 3).await.unwrap());
    assert_eq!(store.get_ticket("par").await.unwrap().unwrap().summary.stage, TicketStage::InProgress);

    store.start_attempt("at", "a", None, "b", "main", "c", json!({}), 4).await.unwrap();
    let rows = store.part_rows(&["par".into()]).await.unwrap();
    assert_eq!(rows[0].attempt, Some(("at".into(), AttemptState::Active)));
}

#[tokio::test]
async fn parts_copy_their_parents_areas_and_parents_have_no_overlap() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "par", ValidNewTicket { areas: vec!["web".into()], ..new("Parent") }).await;
    let a = add(&store, "a", part("A", "par")).await;
    assert_eq!(a.summary.areas, ["web"]);
    let b = add(&store, "b", ValidNewTicket { areas: vec!["api".into()], ..part("B", "par") }).await;
    assert_eq!(b.summary.areas, ["api"], "given areas win");
    store.start_attempt("at", "a", None, "br", "main", "c", json!({}), 2).await.unwrap();
    let rows = store.overlap_rows(Some("p")).await.unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.ticket_id.as_str()).collect();
    assert!(!ids.contains(&"par"), "a parent is not compared: {ids:?}");
    assert!(ids.contains(&"a") && ids.contains(&"b"));
}
