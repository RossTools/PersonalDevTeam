use super::*;

#[tokio::test]
async fn numbers_count_up_per_project_and_are_never_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let a = add(&store, "a", new("A")).await;
    let b = add(&store, "b", new("B")).await;
    assert_eq!((a.summary.number, b.summary.number), (1, 2));
    assert_eq!(a.summary.key, "PX-1");
    assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::Deleted);
    assert_eq!(store.delete_ticket("b").await.unwrap(), DeleteTicket::NotFound);
    assert_eq!(add(&store, "c", new("C")).await.summary.number, 3);

    let q = Project {
        id: "q".into(),
        name: "Other".into(),
        repo_path: "/q".into(),
        base_branch: "main".into(),
        created_at: 2,
        key: "OTH".into(),
        forge: None,
        kind: Default::default(),
    };
    store.insert_project(&q).await.unwrap();
    let t = store.create_ticket("q1", "q", &new("Q"), 1).await.unwrap();
    assert_eq!(t.summary.key, "OTH-1");
    assert!(matches!(store.create_ticket("x", "nope", &new("X"), 1).await, Err(TicketError::NotFound("project"))));

    // Removing the project removes its tickets.
    assert!(store.delete_project("q").await.unwrap());
    assert!(store.get_ticket("q1").await.unwrap().is_none());
}

#[tokio::test]
async fn the_pipeline_stage_is_set_and_any_move_clears_it() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "a", new("A")).await;
    assert!(store.set_pipeline_stage("a", Some(PipelineStage::Refine)).await.unwrap());
    assert!(!store.set_pipeline_stage("nope", Some(PipelineStage::Refine)).await.unwrap());
    let t = store.get_ticket_summary("a").await.unwrap().unwrap();
    assert_eq!(t.pipeline_stage, Some(PipelineStage::Refine));
    store.move_ticket_by("a", TicketStage::Ready, Actor::System, None, 2).await.unwrap();
    assert_eq!(store.get_ticket_summary("a").await.unwrap().unwrap().pipeline_stage, None);
    assert_eq!(store.count_active_attempts("p").await.unwrap(), 0);
    store.start_attempt("at1", "a", None, "b", "main", "abc", json!({}), 3).await.unwrap();
    assert_eq!(store.count_active_attempts("p").await.unwrap(), 1);
    store.set_pipeline_stage("a", Some(PipelineStage::Review)).await.unwrap();
    let page = store.query_tickets(&filter(in_project())).await.unwrap();
    assert_eq!(page.items[0].pipeline_stage, Some(PipelineStage::Review));
}

#[tokio::test]
async fn new_tickets_go_to_the_top_of_their_stage_or_where_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "r1", ValidNewTicket { stage: TicketStage::Ready, ..new("R1") }).await;
    add(&store, "d1", new("D1")).await;
    add(&store, "r2", ValidNewTicket { stage: TicketStage::Ready, ..new("R2") }).await;
    add(&store, "d2", new("D2")).await;
    // A stage's first ticket goes to the top of the whole backlog.
    assert_eq!(ids(&store, in_project()).await, ["d2", "d1", "r2", "r1"]);
    add(&store, "x", ValidNewTicket { before: Some("d1".into()), ..new("X") }).await;
    add(&store, "y", ValidNewTicket { after: Some("r2".into()), ..new("Y") }).await;
    assert_eq!(ids(&store, in_project()).await, ["d2", "x", "d1", "r2", "y", "r1"]);
    let bad = ValidNewTicket { before: Some("d2".into()), after: Some("d1".into()), ..new("Z") };
    assert!(matches!(store.create_ticket("z", "p", &bad, 1).await, Err(TicketError::Invalid(_))));
    let bad = ValidNewTicket { before: Some("nope".into()), ..new("Z") };
    assert!(matches!(store.create_ticket("z", "p", &bad, 1).await, Err(TicketError::Invalid(_))));
    assert!(store.get_ticket("z").await.unwrap().is_none(), "a refused create leaves nothing");
}

#[tokio::test]
async fn moving_writes_one_rank_and_rebalances_long_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["c", "b", "a"] {
        add(&store, id, new(id)).await;
    }
    assert_eq!(ids(&store, in_project()).await, ["a", "b", "c"]);
    store.move_ticket("a", None, Some("c"), None).await.unwrap().unwrap();
    assert_eq!(ids(&store, in_project()).await, ["b", "c", "a"]);
    store.move_ticket("a", Some("c"), None, None).await.unwrap().unwrap();
    assert_eq!(ids(&store, in_project()).await, ["b", "a", "c"]);
    store.move_ticket("c", None, None, Some(RankEdge::Top)).await.unwrap().unwrap();
    assert_eq!(ids(&store, in_project()).await, ["c", "b", "a"]);
    store.move_ticket("c", None, None, Some(RankEdge::Bottom)).await.unwrap().unwrap();
    assert_eq!(ids(&store, in_project()).await, ["b", "a", "c"]);
    assert!(store.move_ticket("nope", Some("a"), None, None).await.unwrap().is_none());
    assert!(matches!(store.move_ticket("a", Some("a"), None, None).await, Err(TicketError::Invalid(_))));
    assert!(matches!(store.move_ticket("a", None, None, None).await, Err(TicketError::Invalid(_))));

    // Swap the last two over and over: the same gap is split each time, until a rebalance resets the keys.
    for i in 0..400 {
        let (x, y) = if i % 2 == 0 { ("c", "a") } else { ("a", "c") };
        store.move_ticket(x, Some(y), None, None).await.unwrap();
    }
    let page = store.query_tickets(&filter(in_project())).await.unwrap();
    assert_eq!(page.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
    assert!(page.items.iter().all(|t| t.rank.len() <= rank::REBALANCE_LEN), "{:?}", page.items);
}
