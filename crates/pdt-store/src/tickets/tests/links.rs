use super::*;

#[tokio::test]
async fn an_external_issue_links_to_one_ticket() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    // Whatever the caller says, an import's link is `imported` and Done closes its issue.
    let link = StoredExternalLink::new("", "tracker", "c", "ABC-1", "https://s/browse/ABC-1", LinkOrigin::Manual);
    let t = store.create_linked_ticket("t1", "p", &new("Fix"), &link, Actor::System, 1).await.unwrap().unwrap();
    assert_eq!(t.summary.source, TicketSource::External, "what an issue says is outside input");
    assert_eq!(
        t.summary.external,
        [ExternalLink {
            system: "tracker".into(),
            key: "ABC-1".into(),
            url: link.url.clone(),
            origin: LinkOrigin::Imported,
            close_on_done: true,
            title: None,
            state: None,
        }]
    );
    assert_eq!(
        store.create_linked_ticket("t2", "p", &new("Again"), &link, Actor::System, 2).await.unwrap(),
        Err("t1".into())
    );
    assert!(store.get_ticket("t2").await.unwrap().is_none(), "nothing made the second time");
    assert_eq!(store.ticket_for_external("tracker", "c", "ABC-1").await.unwrap().as_deref(), Some("t1"));
    assert_eq!(store.ticket_for_external("tracker", "other", "ABC-1").await.unwrap(), None);
    assert_eq!(store.external_links("t1", "tracker").await.unwrap()[0].connection_id, "c");
    let page = store.query_tickets(&filter(in_project())).await.unwrap();
    assert_eq!(page.items[0].external[0].key, "ABC-1", "lists carry the link too");
    store.delete_ticket("t1").await.unwrap();
    assert_eq!(store.ticket_for_external("tracker", "c", "ABC-1").await.unwrap(), None, "gone with its ticket");
}

#[tokio::test]
async fn a_ticket_filed_as_an_issue_links_it_and_stays_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    let t = add(&store, "t1", new("Fix")).await;
    let link = StoredExternalLink::new(
        &t.summary.id,
        "github",
        "o/r",
        "o/r#5",
        "https://github.com/o/r/issues/5",
        LinkOrigin::Created,
    );
    let linked = store.link_external(&link, Actor::User, 9).await.unwrap().unwrap();
    assert_eq!(linked.summary.source, TicketSource::User, "filed from the ticket: not outside input");
    assert_eq!(
        linked.summary.external,
        [ExternalLink {
            system: "github".into(),
            key: "o/r#5".into(),
            url: link.url.clone(),
            origin: LinkOrigin::Created,
            close_on_done: true,
            title: None,
            state: None,
        }]
    );
    assert_eq!(linked.summary.updated_at, 9);
    assert_eq!(events(&store, "t1").await, [EventKind::Created, EventKind::Linked]);
    let other = add(&store, "t2", new("Other")).await;
    let again = StoredExternalLink { ticket_id: other.summary.id, ..link.clone() };
    assert_eq!(store.link_external(&again, Actor::User, 10).await.unwrap(), Err("t1".into()), "one ticket per issue");
    let gone = StoredExternalLink { ticket_id: "nope".into(), key: "o/r#6".into(), ..link };
    assert!(matches!(store.link_external(&gone, Actor::User, 10).await, Err(TicketError::NotFound("ticket"))));
}

#[tokio::test]
async fn an_issue_linked_by_hand_is_closed_on_done_only_when_asked() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "t1", new("Fix")).await;
    let mut link =
        StoredExternalLink::new("t1", "github", "o/r", "o/r#7", "https://github.com/o/r/issues/7", LinkOrigin::Manual);
    assert!(!link.close_on_done, "someone else's issue is never closed by surprise");
    (link.title, link.state) = (Some("Crash".into()), Some(IssueState::Open));
    let t = store.link_external(&link, Actor::User, 5).await.unwrap().unwrap();
    let l = &t.summary.external[0];
    assert_eq!((l.origin, l.close_on_done), (LinkOrigin::Manual, false));
    assert_eq!((l.title.as_deref(), l.state), (Some("Crash"), Some(IssueState::Open)));
    assert!(!store.external_links("t1", "github").await.unwrap()[0].close_on_done, "Done leaves it alone");
    assert_eq!(store.issue_links("t1").await.unwrap().len(), 1);
    assert!(store.issue_links_to_read("t1", 4).await.unwrap().is_empty(), "read when it was linked");
    assert_eq!(store.issue_links_to_read("t1", 5).await.unwrap().len(), 1);

    let t = store.set_close_on_done("t1", "github", "o/r#7", true, 6).await.unwrap();
    assert!(t.summary.external[0].close_on_done);
    assert!(store.external_links("t1", "github").await.unwrap()[0].close_on_done);
    assert!(matches!(
        store.set_close_on_done("t1", "github", "o/r#8", true, 6).await,
        Err(TicketError::NotFound("link"))
    ));

    store.issue_seen(&link, "Crash on start", IssueState::Closed, 7).await.unwrap();
    let t = store.get_ticket("t1").await.unwrap().unwrap();
    assert_eq!(t.summary.external[0].title.as_deref(), Some("Crash on start"));
    assert_eq!(t.summary.external[0].state, Some(IssueState::Closed));

    let t = store.unlink_external("t1", "github", "o/r#7", Actor::User, 8).await.unwrap();
    assert!(t.summary.external.is_empty());
    assert_eq!(events(&store, "t1").await, [EventKind::Created, EventKind::Linked, EventKind::Unlinked]);
    assert!(matches!(
        store.unlink_external("t1", "github", "o/r#7", Actor::User, 9).await,
        Err(TicketError::NotFound("link"))
    ));
    assert!(matches!(
        store.unlink_external("nope", "github", "o/r#7", Actor::User, 9).await,
        Err(TicketError::NotFound("ticket"))
    ));
}

#[tokio::test]
async fn plain_links_are_kept_apart_from_issues() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    add(&store, "t1", new("Fix")).await;
    store.add_web_link("t1", "https://figma.com/f/1", "Figma", Actor::User, 2).await.unwrap();
    let t = store.add_web_link("t1", "https://docs.rs/x", "Docs", Actor::User, 3).await.unwrap();
    assert_eq!(t.web_links.iter().map(|l| l.label.as_str()).collect::<Vec<_>>(), ["Figma", "Docs"]);
    assert!(t.summary.external.is_empty());
    let t = store.add_web_link("t1", "https://figma.com/f/1", "Design", Actor::User, 4).await.unwrap();
    assert_eq!(t.web_links.len(), 2, "the same address is relabelled, not added twice");
    assert_eq!(t.web_links[0].label, "Design");
    let id = t.web_links[1].id.clone();
    let t = store.remove_web_link("t1", &id, Actor::User, 5).await.unwrap();
    assert_eq!(t.web_links.len(), 1);
    assert!(matches!(store.remove_web_link("t1", &id, Actor::User, 6).await, Err(TicketError::NotFound("link"))));
    assert!(matches!(
        store.add_web_link("nope", "https://a.b", "A", Actor::User, 6).await,
        Err(TicketError::NotFound("ticket"))
    ));
}

#[tokio::test]
async fn links_refuse_cycles_and_show_on_both_tickets() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["a", "b", "c"] {
        add(&store, id, new(id)).await;
    }
    store.add_link("a", "b", LinkKind::Blocks, 1).await.unwrap();
    store.add_link("b", "c", LinkKind::Blocks, 1).await.unwrap();
    match store.add_link("c", "a", LinkKind::Blocks, 1).await {
        Err(TicketError::Cycle(keys)) => assert_eq!(keys, ["PX-3", "PX-1", "PX-2", "PX-3"]),
        other => panic!("{other:?}"),
    }
    assert!(matches!(store.add_link("a", "a", LinkKind::Relates, 1).await, Err(TicketError::Invalid(_))));
    assert!(matches!(store.add_link("a", "nope", LinkKind::Relates, 1).await, Err(TicketError::Invalid(_))));
    assert!(store.add_link("nope", "a", LinkKind::Relates, 1).await.unwrap().is_none());

    store.add_link("c", "a", LinkKind::Relates, 1).await.unwrap();
    let t = store.add_link("a", "c", LinkKind::Relates, 1).await.unwrap().unwrap();
    let summary: Vec<(LinkKind, LinkDirection, &str)> =
        t.links.iter().map(|l| (l.kind, l.direction, l.key.as_str())).collect();
    assert_eq!(
        summary,
        [(LinkKind::Blocks, LinkDirection::Outgoing, "PX-2"), (LinkKind::Relates, LinkDirection::Incoming, "PX-3")]
    );
    assert_eq!(store.get_ticket("b").await.unwrap().unwrap().links.len(), 2);

    // relates comes off from either side; blocks only as it was made.
    store.remove_link("a", "c", LinkKind::Relates, 2).await.unwrap();
    assert!(matches!(store.remove_link("b", "a", LinkKind::Blocks, 2).await, Err(TicketError::NotFound("link"))));
    store.remove_link("a", "b", LinkKind::Blocks, 2).await.unwrap();
    assert_eq!(
        events(&store, "b").await,
        [EventKind::Created, EventKind::Linked, EventKind::Linked, EventKind::Unlinked, EventKind::Unblocked]
    );
    store.delete_ticket("c").await.unwrap();
    assert!(store.get_ticket("b").await.unwrap().unwrap().links.is_empty());
}

#[tokio::test]
async fn linking_and_unlinking_change_both_tickets() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store_with_project(tmp.path()).await;
    for id in ["a", "b"] {
        add(&store, id, new(id)).await;
    }
    let updated = |id: &'static str| {
        let store = &store;
        async move { store.get_ticket_summary(id).await.unwrap().unwrap().updated_at }
    };
    store.add_link("a", "b", LinkKind::Blocks, 9_000).await.unwrap();
    assert_eq!((updated("a").await, updated("b").await), (9_000, 9_000));
    store.remove_link("a", "b", LinkKind::Blocks, 9_500).await.unwrap();
    assert_eq!((updated("a").await, updated("b").await), (9_500, 9_500));
}
