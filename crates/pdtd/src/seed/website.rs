//! The website's sample data (`PDT_SEED=website`, PDT-273): one made-up project, "Acme Shop" (a small web storefront),
//! with a board that has a ticket in every stage, a backlog, sessions, runs and a workflow, for the screenshots
//! `scripts/site-screenshots.mjs` takes. Nothing in it is about PDT itself or any real person, repository or data.
//!
//! The project uses the local forge (`forge = "local"`), so its tickets' change requests and CI show without GitHub:
//! each started ticket's branch has commits, an open change request and a finished CI run on its head, so the forge
//! has nothing to open or run. As in the demo seed, nothing is started: the sessions are finished (the screenshot
//! script restarts two of them on a stand-in program) and the runs are over.

use anyhow::Context;
use pdt_core::conversation::*;
use pdt_core::inbox::{InboxChoice, InboxGroup, InboxItem, InboxKind, InboxStatus};
use pdt_core::pr::CiState;
use pdt_core::project::Project;
use pdt_core::run::{Run, RunKind, RunState};
use pdt_core::session::{AgentKind, Session, SessionKind, SessionStatus};
use pdt_core::ticket::{LinkKind, NewTicket, PipelineStage, Priority, TicketStage};
use pdt_core::workflow::api::{Author, Scope, Source, Workflow};
use pdt_core::workflow::doc::WorkflowDoc;
use pdt_core::worktree::{WORKTREES_DIR, Worktree, worktree_path};
use pdt_store::Store;
use serde_json::json;
use std::path::{Path, PathBuf};

/// Where the sample repository lives, inside the data folder.
pub const WEBSITE_DIR: &str = "website";

/// The sample project's name and ticket key.
pub const PROJECT_NAME: &str = "Acme Shop";
pub const PROJECT_KEY: &str = "SHOP";

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const CONFIG: &str = r#"# Acme Shop's run configuration.
forge = "local"

[ci]
command = "npm test"

[[run]]
name    = "dev"
command = "python3 -m http.server $PORT --directory src"
web     = true

[[test]]
name    = "unit"
command = "npm test"

[[area]]
name  = "checkout"
paths = ["src/checkout/**"]

[[area]]
name  = "cart"
paths = ["src/cart.js"]

[[area]]
name  = "catalog"
paths = ["src/catalog.js", "src/products.json"]

[[area]]
name      = "styles"
paths     = ["src/*.css"]
exclusive = true
"#;

/// One ticket of the sample backlog.
struct Item {
    title: &'static str,
    description: &'static str,
    stage: TicketStage,
    priority: Priority,
    labels: &'static [&'static str],
    areas: &'static [&'static str],
    /// The index of its parent ticket in the list.
    parent: Option<usize>,
    /// Where the pipeline has it (In progress only), and how its change request's CI went.
    work: Option<(PipelineStage, CiState)>,
    /// How long ago it was last touched.
    ago: i64,
}

const fn item(title: &'static str, stage: TicketStage, priority: Priority, ago: i64) -> Item {
    Item { title, description: "", stage, priority, labels: &[], areas: &[], parent: None, work: None, ago }
}

/// The backlog, top to bottom in each stage. Index 0 is the parent of the checkout parts.
fn items() -> Vec<Item> {
    use Priority::*;
    use TicketStage::*;
    vec![
        Item {
            description: "Let people buy without an account, take discount codes, and email a receipt. One PR per part.",
            ..item("Checkout v2", InProgress, High, 2 * HOUR)
        },
        // In progress, one per pipeline stage.
        Item {
            description: "Match product names and tags as you type, and show \"No results\" with a link back to all \
                          products.\n\n## Done when\n- [ ] Typing filters the grid within 100 ms for 500 products\n\
                          - [ ] Tags match too\n- [ ] The empty state links back",
            labels: &["agentic", "frontend"],
            work: Some((PipelineStage::Code, CiState::Passed)),
            ..item("Search products by name", InProgress, High, 4 * MINUTE)
        },
        Item {
            description: "Send a receipt with the order's items, total and delivery address once payment clears.",
            labels: &["agentic", "backend"],
            parent: Some(0),
            work: Some((PipelineStage::Code, CiState::Failed)),
            ..item("Order confirmation email", InProgress, Medium, 12 * MINUTE)
        },
        Item {
            description: "Keep the cart in local storage, and merge it into the account's cart on sign-in.",
            labels: &["agentic", "frontend"],
            areas: &["cart"],
            work: Some((PipelineStage::Review, CiState::Passed)),
            ..item("Save the cart across visits", InProgress, Medium, 7 * MINUTE)
        },
        Item {
            description: "An email address and a delivery address are enough to place an order.",
            labels: &["agentic", "checkout"],
            parent: Some(0),
            work: Some((PipelineStage::Qa, CiState::Passed)),
            ..item("Guest checkout without an account", InProgress, High, 3 * MINUTE)
        },
        Item {
            description: "A code field under the total; an invalid or expired code says why.",
            labels: &["checkout"],
            parent: Some(0),
            work: Some((PipelineStage::Accept, CiState::Passed)),
            ..item("Apply discount codes at checkout", InProgress, High, 25 * MINUTE)
        },
        Item {
            description: "\"Only 3 left\" under the price when stock is low; \"Out of stock\" disables Add to cart.",
            labels: &["agentic", "frontend"],
            work: Some((PipelineStage::Merge, CiState::Passed)),
            ..item("Show stock levels on product pages", InProgress, Medium, 9 * MINUTE)
        },
        // To do.
        Item {
            description: "A heart on each product card; the list is under Account → Wishlist.",
            labels: &["agentic", "frontend"],
            ..item("Wishlist for signed-in customers", Todo, Medium, 3 * HOUR)
        },
        Item {
            description: "Flat rates per region from a table admins can edit.",
            labels: &["backend", "checkout"],
            ..item("Shipping rates by region", Todo, High, 5 * HOUR)
        },
        Item {
            description: "Labels, focus order and error messages a screen reader announces.",
            labels: &["agentic", "a11y"],
            ..item("Accessibility pass on the checkout forms", Todo, Medium, 6 * HOUR)
        },
        // The backlog.
        Item {
            description: "A cart of three items at 19.99 shows 59.96. Round once, on the total.",
            labels: &["bug", "cart"],
            areas: &["cart"],
            ..item("Fix price rounding in cart totals", Ready, Urgent, 40 * MINUTE)
        },
        Item {
            description: "Behind the payment provider's button; falls back to card entry.",
            labels: &["checkout"],
            ..item("Pay with Apple Pay", Ready, Medium, 8 * HOUR)
        },
        Item {
            labels: &["admin", "backend"],
            description: "Date range, status filter; one row per order line.",
            ..item("Export orders as CSV", Ready, Low, DAY)
        },
        Item {
            description: "Email the shop owner when a product drops below its reorder level.",
            labels: &["admin"],
            ..item("Low-stock alerts for admins", Ready, Medium, DAY + 3 * HOUR)
        },
        Item {
            labels: &["frontend", "performance"],
            description: "Only the first row of the grid loads eagerly.",
            ..item("Lazy-load product images", Ready, Low, 2 * DAY)
        },
        Item {
            description: "Stars and a short review, from customers who bought the product.",
            labels: &["frontend", "backend"],
            ..item("Product reviews and ratings", Draft, Medium, 2 * DAY)
        },
        Item { labels: &["checkout"], ..item("Gift cards", Draft, Low, 3 * DAY) },
        Item { labels: &["i18n"], ..item("Translate the storefront into German", Draft, Low, 4 * DAY) },
        Item { labels: &["frontend"], ..item("Recently viewed products", Draft, None, 4 * DAY) },
        Item {
            labels: &["security", "backend"],
            description: "Five tries a minute per address, then a short wait.",
            ..item("Rate-limit the sign-in endpoint", Draft, High, 5 * DAY)
        },
        Item { labels: &["frontend"], ..item("Dark theme for the storefront", Draft, None, 6 * DAY) },
        // Done lately.
        Item { labels: &["frontend"], ..item("Filter products by category", Done, Medium, 5 * HOUR) },
        Item { labels: &["agentic"], ..item("Product images in the catalog grid", Done, Medium, DAY) },
        Item { labels: &["ci"], ..item("Run the unit tests on every change", Done, Low, 2 * DAY) },
    ]
}

/// Seeds the website's sample data unless the database already has a project. Returns whether it seeded. `now` is
/// in ms.
pub async fn seed_website(store: &Store, data_dir: &Path, now: i64) -> anyhow::Result<bool> {
    if !store.list_projects().await?.is_empty() {
        return Ok(false);
    }
    let root = data_dir.join(WEBSITE_DIR);
    let repo = tokio::task::spawn_blocking(move || make_repo(&root)).await??;
    let id = || uuid::Uuid::now_v7().to_string();
    let path = |p: &Path| p.to_string_lossy().into_owned();
    let project = Project {
        id: id(),
        name: PROJECT_NAME.into(),
        repo_path: path(&repo.root),
        base_branch: "main".into(),
        created_at: now - 30 * DAY,
        key: PROJECT_KEY.into(),
        forge: None,
        kind: Default::default(),
    };
    store.insert_project(&project).await?;

    // The tickets. Each new ticket goes to the top of its stage, so create bottom-up to keep the list's order; parts
    // need their parent first.
    let items = items();
    let mut tickets: Vec<Option<(String, String)>> = vec![None; items.len()];
    let order: Vec<usize> =
        items.iter().enumerate().filter(|(_, i)| i.parent.is_none()).map(|(n, _)| n).rev().collect::<Vec<_>>();
    let parts: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.parent.is_some()).map(|(n, _)| n).collect();
    for n in order.into_iter().chain(parts) {
        let it = &items[n];
        let new = NewTicket {
            title: it.title.into(),
            description: Some(it.description.into()),
            stage: Some(it.stage),
            priority: Some(it.priority),
            labels: Some(it.labels.iter().map(|l| (*l).to_string()).collect()),
            areas: Some(it.areas.iter().map(|a| (*a).to_string()).collect()),
            parent: it.parent.and_then(|p| tickets[p].as_ref().map(|(id, _)| id.clone())),
            ..Default::default()
        };
        let valid = new.validate().context("sample ticket")?;
        let t = store.create_ticket(&id(), &project.id, &valid, now - it.ago).await?;
        tickets[n] = Some((t.summary.id, t.summary.key));
    }
    let tickets: Vec<(String, String)> = tickets.into_iter().map(|t| t.expect("every ticket is made")).collect();
    let by_title = |title: &str| {
        let n = items.iter().position(|i| i.title == title).expect("a sample ticket");
        tickets[n].clone()
    };

    // Links and comments.
    let (guest, _) = by_title("Guest checkout without an account");
    let (apple, _) = by_title("Pay with Apple Pay");
    store.add_link(&guest, &apple, LinkKind::Blocks, now - DAY).await?;
    let (shipping, _) = by_title("Shipping rates by region");
    store.add_link(&guest, &shipping, LinkKind::Blocks, now - DAY).await?;
    let (rounding, _) = by_title("Fix price rounding in cart totals");
    store
        .add_comment(&rounding, "Seen by two customers this week; the receipt shows the right total.", now - HOUR)
        .await?;

    // Started tickets: a worktree with the agent's commits, an attempt, an open change request with its CI.
    let started: Vec<(usize, String)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.work.is_some())
        .map(|(n, i)| (n, branch_name(&tickets[n].1, i.title)))
        .collect();
    let branches: Vec<(String, Vec<(&'static str, &'static str)>)> =
        started.iter().map(|(n, b)| (b.clone(), change_for(items[*n].title))).collect();
    let repo_root = repo.root.clone();
    let heads = tokio::task::spawn_blocking(move || make_branches(&repo_root, &branches)).await??;
    let mut attempts = Vec::new();
    for ((n, branch), head) in started.iter().zip(heads) {
        let it = &items[*n];
        let (ticket_id, key) = &tickets[*n];
        let (stage, ci) = it.work.expect("started");
        let at = now - it.ago - 3 * HOUR;
        let wt = Worktree {
            id: id(),
            project_id: project.id.clone(),
            branch: branch.clone(),
            path: path(&head.path),
            base_commit: head.base.clone(),
            created_at: at,
            missing: false,
        };
        store.insert_worktree(&wt).await?;
        let attempt = id();
        store.insert_attempt(&attempt, ticket_id, Some(&wt.id), branch, "main", &head.base, at).await?;
        store.set_pipeline_stage(ticket_id, Some(stage)).await?;
        let change = store
            .open_local_change(&project.id, branch, "main", &format!("{key}: {}", it.title), &head.head, at + HOUR)
            .await?;
        let run = store.add_local_ci_run(change.id, &head.head, "npm test", now - it.ago - 6 * MINUTE).await?;
        let code = if ci == CiState::Passed { 0 } else { 1 };
        store.finish_local_ci_run(run.id, ci, Some(code), now - it.ago - 4 * MINUTE).await?;
        attempts.push((it.title, ticket_id.clone(), key.clone(), attempt, wt));
    }
    let attempt_of = |title: &str| attempts.iter().find(|a| a.0 == title).expect("a started ticket");

    // The ticket waiting for the user's Accept: its gate in the Inbox ("Your turn" on the card).
    let (_, accept_id, accept_key, accept_attempt, _) = attempt_of("Apply discount codes at checkout");
    let question = format!("{accept_key} passed review and QA. Accept it?");
    store
        .open_inbox_item(&InboxItem {
            id: id(),
            kind: InboxKind::Gate,
            group: InboxGroup::Accept,
            status: InboxStatus::Open,
            project_id: Some(project.id.clone()),
            ticket_id: Some(accept_id.clone()),
            ticket_key: None,
            ticket_title: None,
            attempt_id: Some(accept_attempt.clone()),
            session_id: None,
            run_id: None,
            step_id: None,
            persona_id: None,
            persona_name: None,
            change_request: None,
            title: question.clone(),
            body: json!({"question": question, "workflow": "Pipeline", "node": "Accept"}),
            choices: vec![InboxChoice::new("accept", "Accept", true), InboxChoice::new("reject", "Send back", false)],
            resolution: serde_json::Value::Null,
            created_at: now - 20 * MINUTE,
            resolved_at: None,
        })
        .await?;

    // Sessions, each a tab of the project's workspace (titled by persona: the ticket's key shows beside them): the
    // Coder's conversation on the search ticket, a Reviewer and a QA agent (the screenshot script brings these two back on a stand-in), and a shell.
    let session = |wt: Option<&Worktree>, agent: Option<AgentKind>, title: String, ago: i64| Session {
        id: id(),
        project_id: project.id.clone(),
        worktree_id: wt.map(|w| w.id.clone()),
        kind: if agent.is_some() { SessionKind::AgentTui } else { SessionKind::Shell },
        agent,
        bypass_permissions: false,
        title,
        cwd: wt.map_or_else(|| project.repo_path.clone(), |w| w.path.clone()),
        status: SessionStatus::Finished,
        exit_code: Some(0),
        created_at: now - ago,
        ended_at: Some(now - ago + 20 * MINUTE),
        external_session_id: None,
        fresh_started_at: None,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: None,
        ticket: None,
        mode: SessionMode::Terminal,
        model: None,
        permission_mode: None,
        persona: None,
    };
    let (_, _, _, search_attempt, search_wt) = attempt_of("Search products by name");
    let coder = Session {
        mode: SessionMode::Conversation,
        permission_mode: Some(PermissionMode::Ask),
        exit_code: None,
        attempt_id: Some(search_attempt.clone()),
        ..session(Some(search_wt), Some(AgentKind::Claude), "Coder".into(), 30 * MINUTE)
    };
    let (_, _, _, cart_attempt, cart_wt) = attempt_of("Save the cart across visits");
    let reviewer = Session {
        attempt_id: Some(cart_attempt.clone()),
        ..session(Some(cart_wt), Some(AgentKind::Claude), "Reviewer".into(), 20 * MINUTE)
    };
    let (_, _, _, guest_attempt, guest_wt) = attempt_of("Guest checkout without an account");
    let qa = Session {
        attempt_id: Some(guest_attempt.clone()),
        ..session(Some(guest_wt), Some(AgentKind::Claude), "QA".into(), 15 * MINUTE)
    };
    let shell = session(None, None, "main".into(), 2 * HOUR);
    for s in [&shell, &coder, &reviewer, &qa] {
        store.insert_session(s).await?;
        if let Some(home) = store.project_workspace_id(&s.project_id).await? {
            store
                .open_tab(&id(), &home, &pdt_core::workspace::TabItem::Session(s.id.clone()), None, s.created_at)
                .await?;
        }
    }
    seed_conversation(store, &coder.id, &search_wt.branch, now - 30 * MINUTE).await?;

    let run = |wt: Option<&Worktree>, name: &str, kind: RunKind, state: RunState, ago: i64| Run {
        id: id(),
        project_id: project.id.clone(),
        worktree_id: wt.map(|w| w.id.clone()),
        branch: wt.map_or_else(|| "main".into(), |w| w.branch.clone()),
        name: name.into(),
        kind,
        state,
        port: None,
        url: None,
        data_dir: None,
        cwd: wt.map_or_else(|| project.repo_path.clone(), |w| w.path.clone()),
        exit_code: Some(if state == RunState::Failed { 1 } else { 0 }),
        error: (state == RunState::Failed).then(|| "tests failed (exit code 1)".into()),
        detached: false,
        process: None,
        created_at: now - ago,
        updated_at: now - ago + MINUTE,
    };
    for r in [
        run(None, "unit", RunKind::Test, RunState::Passed, 3 * HOUR),
        run(Some(guest_wt), "unit", RunKind::Test, RunState::Passed, 10 * MINUTE),
    ] {
        store.insert_run(&r).await?;
    }

    // The user's line comment on the cart branch, sent to the agent on it, and the branch's changes open beside the
    // sessions (the review screenshot).
    let line = CART_JS.lines().position(|l| l == REVIEWED_LINE).expect("the reviewed line") + 1;
    let line = u32::try_from(line)?;
    store
        .insert_review_comment(&pdt_core::review::ReviewComment {
            id: id(),
            worktree_id: cart_wt.id.clone(),
            parent_id: None,
            path: "src/cart.js".into(),
            side: pdt_core::review::CommentSide::New,
            line,
            end_line: line,
            line_text: REVIEWED_LINE.into(),
            body: "Adding the two quantities can ask for more than we have in stock. Cap the merged line at the \
                   product's stock, and add a test for it."
                .into(),
            author: "user".into(),
            sent_at: Some(now - 18 * MINUTE),
            session_id: Some(reviewer.id.clone()),
            resolved_at: None,
            created_at: now - 18 * MINUTE,
            updated_at: now - 18 * MINUTE,
        })
        .await?;
    if let Some(home) = store.project_workspace_id(&project.id).await? {
        let changes = pdt_core::workspace::TabItem::Changes(cart_wt.id.clone());
        store.open_tab(&id(), &home, &changes, None, now - 18 * MINUTE).await?;
    }

    // A question from the Coder on the receipt ticket, beside the Accept gate in the Inbox.
    let (_, email_id, _, email_attempt, _) = attempt_of("Order confirmation email");
    let ask = "Should the receipt attach a PDF invoice, or is the email's text enough?";
    store
        .open_inbox_item(&InboxItem {
            id: id(),
            kind: InboxKind::Question,
            group: InboxGroup::Question,
            status: InboxStatus::Open,
            project_id: Some(project.id.clone()),
            ticket_id: Some(email_id.clone()),
            ticket_key: None,
            ticket_title: None,
            attempt_id: Some(email_attempt.clone()),
            session_id: None,
            run_id: None,
            step_id: None,
            persona_id: None,
            persona_name: None,
            change_request: None,
            title: ask.into(),
            body: json!({"questions": [{"question": ask, "choices": ["Attach a PDF", "Text is enough"]}]}),
            choices: vec![],
            resolution: serde_json::Value::Null,
            created_at: now - 10 * MINUTE,
            resolved_at: None,
        })
        .await?;

    seed_pm(store, data_dir, &by_title, now - 5 * MINUTE).await?;
    seed_workflow(store, &project.id, now - 7 * DAY).await?;
    Ok(true)
}

/// A ticket's branch, as Start names it: `pdt-<KEY>-<n>-<slug>`.
fn branch_name(key: &str, title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(5)
        .collect::<Vec<_>>()
        .join("-");
    format!("pdt-{key}-{slug}")
}

/// The files an agent changed on each started ticket's branch, with their new text.
fn change_for(title: &str) -> Vec<(&'static str, &'static str)> {
    match title {
        "Search products by name" => vec![("src/catalog.js", CATALOG_JS)],
        "Order confirmation email" => vec![("src/checkout/receipt.js", RECEIPT_JS)],
        "Save the cart across visits" => vec![("src/cart.js", CART_JS), ("test/cart.test.js", CART_TEST_JS)],
        "Guest checkout without an account" => vec![("src/checkout/guest.js", GUEST_JS)],
        "Apply discount codes at checkout" => vec![("src/checkout/discounts.js", DISCOUNTS_JS)],
        _ => vec![("src/stock.js", STOCK_JS)],
    }
}

/// The search branch's catalog: the Coder's conversation adds `search()`.
const CATALOG_JS: &str = r#"export function render(products, grid) {
  grid.replaceChildren(...products.map(card));
}

/** Products whose name or tags contain every word of the query. */
export function search(products, query) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return products;
  return products.filter((p) => {
    const text = [p.name, ...p.tags].join(' ').toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

export function card(p) {
  const el = document.createElement('article');
  el.textContent = `${p.name} · ${p.price}`;
  return el;
}
"#;

/// The cart branch's change, which the review screenshot shows with the user's comment on [`REVIEWED_LINE`].
const CART_JS: &str = r#"const KEY = 'acme.cart';

/** The cart as the last visit left it (empty when there is none, or it can't be read). */
export function loadCart(storage = localStorage) {
  try {
    return JSON.parse(storage.getItem(KEY)) ?? [];
  } catch {
    return [];
  }
}

export function saveCart(cart, storage = localStorage) {
  storage.setItem(KEY, JSON.stringify(cart));
}

/** Merges the saved cart into the account's on sign-in: a product in both keeps both quantities. */
export function mergeCarts(saved, account) {
  const merged = new Map(account.map((line) => [line.sku, { ...line }]));
  for (const line of saved) {
    const had = merged.get(line.sku);
    if (had) had.qty += line.qty;
    else merged.set(line.sku, { ...line });
  }
  return [...merged.values()];
}

export const cart = loadCart();
"#;

/// The line of [`CART_JS`] the user commented on.
const REVIEWED_LINE: &str = "    if (had) had.qty += line.qty;";

const CART_TEST_JS: &str = r#"import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadCart, mergeCarts, saveCart } from '../src/cart.js';

const memory = () => {
  const m = new Map();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => m.set(k, v) };
};

test('a saved cart comes back', () => {
  const s = memory();
  saveCart([{ sku: 'mug', qty: 2 }], s);
  assert.deepEqual(loadCart(s), [{ sku: 'mug', qty: 2 }]);
});

test('signing in merges the two carts', () => {
  const merged = mergeCarts([{ sku: 'mug', qty: 1 }], [{ sku: 'mug', qty: 2 }, { sku: 'tee', qty: 1 }]);
  assert.deepEqual(merged, [{ sku: 'mug', qty: 3 }, { sku: 'tee', qty: 1 }]);
});
"#;

const RECEIPT_JS: &str = r#"/** The receipt email for a paid order. */
export function receipt(order) {
  const lines = order.items.map((i) => `${i.qty} × ${i.name}  ${i.total}`);
  return {
    to: order.email,
    subject: `Your Acme Shop order ${order.number}`,
    text: [...lines, '', `Total: ${order.total}`, `Delivering to: ${order.address}`].join('\n'),
  };
}
"#;

const GUEST_JS: &str = r#"/** An order from someone without an account: an email and a delivery address are enough. */
export function guestOrder(cart, { email, address }) {
  if (!/^[^@\s]+@[^@\s]+$/.test(email)) throw new Error('Enter an email address for the receipt');
  if (!address?.trim()) throw new Error('Enter a delivery address');
  return { items: cart, email, address, guest: true };
}
"#;

const DISCOUNTS_JS: &str = r#"/** Applies a discount code to a total, or says why it can't. */
export function applyCode(total, code, codes, now = Date.now()) {
  const c = codes[code.trim().toUpperCase()];
  if (!c) return { total, error: 'That code is not valid' };
  if (c.expires < now) return { total, error: 'That code has expired' };
  return { total: Math.max(0, total - c.amount), code: code.toUpperCase() };
}
"#;

const STOCK_JS: &str = r#"/** What a product page says under the price. */
export function stockLabel(stock) {
  if (stock <= 0) return { text: 'Out of stock', canAdd: false };
  if (stock <= 3) return { text: `Only ${stock} left`, canAdd: true };
  return { text: '', canAdd: true };
}
"#;

/// The Coder's conversation on the search ticket: PDT's prompt, the agent reading and changing the catalog, its
/// tests passing, and its answer.
async fn seed_conversation(store: &Store, session_id: &str, branch: &str, at: i64) -> anyhow::Result<()> {
    let turn = "site-turn-1";
    let diff = "--- a/src/catalog.js\n+++ b/src/catalog.js\n@@ -12,6 +12,17 @@ export function render(products, grid) {\n   grid.replaceChildren(...products.map(card));\n }\n \n+/** Products whose name or tags contain every word of the query. */\n+export function search(products, query) {\n+  const words = query.toLowerCase().split(/\\s+/).filter(Boolean);\n+  if (words.length === 0) return products;\n+  return products.filter((p) => {\n+    const text = [p.name, ...p.tags].join(' ').toLowerCase();\n+    return words.every((w) => text.includes(w));\n+  });\n+}\n+\n export function card(p) {";
    let tool = |id: &str, tool: &str, title: &str, input: serde_json::Value, output: &str| AgentEvent::ToolCall {
        id: id.into(),
        tool: tool.into(),
        title: title.into(),
        input,
        status: ToolStatus::Done,
        output: Some(output.into()),
        exit_code: None,
        diff: None,
        parent_id: None,
    };
    let message = |id: &str, markdown: &str, phase: MessagePhase| AgentEvent::AgentMessage {
        id: id.into(),
        markdown: markdown.into(),
        phase,
    };
    let mut edit =
        tool("site-edit", "Edit", "src/catalog.js", json!({"file_path": "src/catalog.js"}), "The file was updated.");
    if let AgentEvent::ToolCall { diff: d, .. } = &mut edit {
        *d = Some(diff.into());
    }
    let events: Vec<(&str, AgentEvent)> = vec![
        (
            "turn:site-turn-1",
            AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None },
        ),
        (
            "u:site-1",
            AgentEvent::UserMessage {
                text: "Build this ticket on its branch: match product names and tags as you type, with an empty \
                       state that links back to all products. Run the checks and commit."
                    .into(),
                images: vec![],
                source: MessageSource::Pdt,
                pdt: None,
            },
        ),
        ("m:site-1", message("site-1", "I'll start with how the catalog renders the grid.", MessagePhase::Commentary)),
        (
            "t:site-grep",
            tool("site-grep", "Grep", "render(", json!({"pattern": "render("}), "src/catalog.js:9\nsrc/main.js:21"),
        ),
        (
            "t:site-read",
            tool(
                "site-read",
                "Read",
                "src/catalog.js",
                json!({"file_path": "src/catalog.js"}),
                "export function render(products, grid) { … }",
            ),
        ),
        (
            "m:site-2",
            message(
                "site-2",
                "The grid re-renders from a product list, so search can be a pure filter in front of it. Adding \
                 `search()` with tests first.",
                MessagePhase::Commentary,
            ),
        ),
        ("t:site-edit", edit),
        (
            "t:site-wire",
            tool("site-wire", "Edit", "src/main.js", json!({"file_path": "src/main.js"}), "The file was updated."),
        ),
        (
            "t:site-test",
            tool(
                "site-test",
                "Bash",
                "npm test",
                json!({"command": "npm test"}),
                "> acme-shop@0.4.0 test\n> node --test\n\n✔ search matches names (1.2ms)\n✔ search matches tags \
                 (0.4ms)\n✔ an empty query keeps every product (0.2ms)\nℹ tests 18\nℹ pass 18\nℹ fail 0",
            ),
        ),
        (
            "t:site-commit",
            tool(
                "site-commit",
                "Bash",
                "git commit",
                json!({"command": "git commit -am \"Search products by name and tag\""}),
                &format!(
                    "[{branch} 3f9c2ab] Search products by name and tag\n 3 files changed, 64 insertions(+), 2 deletions(-)"
                ),
            ),
        ),
        (
            "m:site-3",
            message(
                "site-3",
                "Search is in. Typing in the box above the grid now filters it:\n\n- **Names and tags** both match, \
                 every word of the query has to\n- **No results** shows a short message with *Show all products*\n\
                 - Filtering 500 products takes about 2 ms, well under the 100 ms in the ticket\n\nThree new tests \
                 cover it; all 18 pass. Committed on the branch, and the change request's CI passed.",
                MessagePhase::Final,
            ),
        ),
        (
            "usage",
            AgentEvent::Usage {
                context_used: Some(48_600),
                context_window: Some(200_000),
                rate_limits: vec![],
                cost_usd: Some(0.31),
            },
        ),
        ("diff:site-turn-1", AgentEvent::TurnDiff { diff: diff.into() }),
        (
            "turn-end:site-turn-1",
            AgentEvent::Turn {
                phase: TurnPhase::Ended,
                status: Some(TurnStatus::Completed),
                duration_ms: Some(214_000),
                reason: None,
            },
        ),
    ];
    insert_events(store, session_id, turn, at, &events).await
}

/// A turn's events, 15 s apart from `at`.
async fn insert_events(
    store: &Store,
    session_id: &str,
    turn: &str,
    at: i64,
    events: &[(&str, AgentEvent)],
) -> anyhow::Result<()> {
    for (i, (item, event)) in events.iter().enumerate() {
        let ne = pdt_store::NewEvent {
            ts: at + i as i64 * 15_000,
            item_id: Some(item),
            turn_id: Some(turn),
            event,
            raw: None,
        };
        store.upsert_session_event(session_id, ne).await?;
    }
    Ok(())
}

/// A PM conversation (the PM screenshot): the user asks what needs them, and the PM reads the board and the Inbox and
/// answers, naming the sample's tickets by key.
async fn seed_pm(
    store: &Store,
    data_dir: &Path,
    by_title: &dyn Fn(&str) -> (String, String),
    at: i64,
) -> anyhow::Result<()> {
    let dir = data_dir.join("pm");
    std::fs::create_dir_all(&dir)?;
    let folder = dir.to_string_lossy().into_owned();
    let host = store.ensure_host_project(&uuid::Uuid::now_v7().to_string(), &folder, at - DAY).await?;
    let key = |title: &str| by_title(title).1;
    let session = Session {
        id: uuid::Uuid::now_v7().to_string(),
        project_id: host.id.clone(),
        worktree_id: None,
        kind: SessionKind::AgentTui,
        agent: Some(AgentKind::Claude),
        bypass_permissions: false,
        title: "What needs me today?".into(),
        cwd: folder,
        status: SessionStatus::Finished,
        exit_code: None,
        created_at: at,
        ended_at: Some(at + 2 * MINUTE),
        external_session_id: None,
        fresh_started_at: None,
        activity: None,
        attention_at: None,
        last_viewed_at: None,
        unseen: false,
        attempt_id: None,
        ticket: None,
        mode: SessionMode::Conversation,
        model: None,
        permission_mode: Some(PermissionMode::Ask),
        persona: None,
    };
    store.insert_session(&session).await?;
    store.insert_pm_session(&session.id, at).await?;
    let tool = |id: &str, tool: &str, title: &str, input: serde_json::Value, output: &str| AgentEvent::ToolCall {
        id: id.into(),
        tool: tool.into(),
        title: title.into(),
        input,
        status: ToolStatus::Done,
        output: Some(output.into()),
        exit_code: None,
        diff: None,
        parent_id: None,
    };
    let (discounts, email, guest, rounding, checkout) = (
        key("Apply discount codes at checkout"),
        key("Order confirmation email"),
        key("Guest checkout without an account"),
        key("Fix price rounding in cart totals"),
        key("Checkout v2"),
    );
    let answer = format!(
        "Three things wait on you in Acme Shop, most urgent first:\n\n\
         1. **{discounts}** *Apply discount codes at checkout* passed review and QA, and waits for your **Accept**.\n\
         2. **{email}** *Order confirmation email*: its Coder asks whether the receipt should attach a PDF invoice \
         (in your Inbox). Its CI also failed on the last push; the Coder is on it.\n\
         3. **{guest}** *Guest checkout*: QA is waiting on a permission prompt to run the dev server.\n\n\
         Otherwise the board is moving: six tickets in flight, two parts of **{checkout}** *Checkout v2* through \
         review. One thing I'd change: **{rounding}** *Fix price rounding* is urgent and customers have seen it, but \
         it is still in Ready. Shall I move it to To do so the Coder takes it next?"
    );
    let events: Vec<(&str, AgentEvent)> = vec![
        ("turn:pm-1", AgentEvent::Turn { phase: TurnPhase::Started, status: None, duration_ms: None, reason: None }),
        (
            "u:pm-1",
            AgentEvent::UserMessage {
                text: "What needs me today?".into(),
                images: vec![],
                source: MessageSource::Pdt,
                pdt: None,
            },
        ),
        (
            "t:pm-overview",
            tool(
                "pm-overview",
                "mcp__pdt__get_overview",
                "get_overview",
                json!({}),
                "Acme Shop: 6 in flight (Code 2, Review 1, QA 1, Accept 1, Merge 1), 3 to do, 5 ready, 6 draft; 2 open Inbox items",
            ),
        ),
        (
            "t:pm-search",
            tool(
                "pm-search",
                "mcp__pdt__search_tickets",
                "search_tickets",
                json!({"stage": "ready", "priority": "urgent"}),
                &format!("{rounding} Fix price rounding in cart totals (urgent, ready)"),
            ),
        ),
        (
            "m:pm-answer",
            AgentEvent::AgentMessage { id: "pm-answer".into(), markdown: answer, phase: MessagePhase::Final },
        ),
        (
            "turn-end:pm-1",
            AgentEvent::Turn {
                phase: TurnPhase::Ended,
                status: Some(TurnStatus::Completed),
                duration_ms: Some(21_000),
                reason: None,
            },
        ),
    ];
    insert_events(store, &session.id, "pm-turn-1", at, &events).await
}

/// The workflow the Workflows screenshot shows: each night, if packages are outdated, an `agentic` ticket for the
/// Coder to update them, and a note to the user. Small enough to read at the site's size; it only runs at 03:00.
fn workflow_doc(project_id: &str) -> WorkflowDoc {
    serde_json::from_value(json!({
        "name": "Nightly dependency updates",
        "description": "Each night, checks for outdated packages. If there are any, files an agentic ticket to \
                        update them (the Pipeline's Coder takes it) and tells you.",
        "nodes": [
            {"id": "nightly", "kind": "schedule", "version": 1, "position": {"x": 0, "y": 100},
             "title": "Every night at 3:00", "config": {"cron": "0 3 * * *", "catchUp": false}},
            {"id": "outdated", "kind": "command", "version": 1, "position": {"x": 260, "y": 100},
             "title": "List outdated packages",
             "config": {"script": "npm outdated || true", "cwd": "project", "parse": "text"}},
            {"id": "any", "kind": "condition", "version": 1, "position": {"x": 520, "y": 100},
             "title": "Anything outdated?",
             "config": {"conditions": [{"left": "{{steps.outdated.output}}", "op": "truthy"}]}},
            {"id": "ticket", "kind": "ticket.create", "version": 1, "position": {"x": 780, "y": 40},
             "title": "Ticket for the Coder",
             "config": {"project": project_id, "title": "Update the outdated packages",
                        "description": "Update each one and run the tests.\n\n```\n{{steps.outdated.output}}\n```",
                        "stage": "todo", "labels": ["agentic", "dependencies"]}},
            {"id": "tell", "kind": "notify", "version": 1, "position": {"x": 1040, "y": 40},
             "title": "Tell me",
             "config": {"title": "Dependency updates are on the board",
                        "body": "{{steps.ticket.key}} updates the outdated packages."}},
            {"id": "done", "kind": "end", "version": 1, "position": {"x": 1300, "y": 40}, "title": "Done"},
            {"id": "skip", "kind": "end", "version": 1, "position": {"x": 780, "y": 200},
             "title": "Nothing to do", "config": {"quiet": true}},
        ],
        "edges": [
            {"from": "nightly", "to": "outdated"},
            {"from": "outdated", "to": "any"},
            {"from": "any", "port": "true", "to": "ticket"},
            {"from": "any", "port": "false", "to": "skip"},
            {"from": "ticket", "to": "tell"},
            {"from": "tell", "to": "done"},
        ],
    }))
    .expect("the sample workflow is a valid document")
}

async fn seed_workflow(store: &Store, project_id: &str, at: i64) -> anyhow::Result<()> {
    let doc = workflow_doc(project_id);
    let w = Workflow {
        id: uuid::Uuid::now_v7().to_string(),
        name: doc.name.clone(),
        description: doc.description.clone(),
        scope: Scope::Project,
        project_id: Some(project_id.to_string()),
        enabled: true,
        builtin: None,
        source: Source::Host,
        repo_file: None,
        version: 1,
        proposed: None,
        created_at: at,
        updated_at: at,
    };
    store.insert_workflow(&w, &doc, Author::User, "").await?;
    Ok(())
}

struct SampleRepo {
    root: PathBuf,
}

/// A started ticket's branch: its worktree, the commit it started from and its head.
struct BranchHead {
    path: PathBuf,
    base: String,
    head: String,
}

/// Builds the sample repository under `root`, replacing whatever an earlier, interrupted seed left there.
fn make_repo(root: &Path) -> anyhow::Result<SampleRepo> {
    if root.exists() {
        std::fs::remove_dir_all(root).with_context(|| format!("clearing {}", root.display()))?;
    }
    let dir = root.join("acme-shop");
    pdt_git::init(&dir, "main")?;
    write(
        &dir,
        "README.md",
        "# Acme Shop\n\nA small web storefront: catalog, cart and checkout.\n\n```sh\nnpm test\n```\n",
    )?;
    write(
        &dir,
        "package.json",
        "{\n  \"name\": \"acme-shop\",\n  \"version\": \"0.4.0\",\n  \"type\": \"module\",\n  \"scripts\": { \"test\": \"node --test\" }\n}\n",
    )?;
    write(&dir, ".pdt/project.toml", CONFIG)?;
    pdt_git::commit_all(&dir, "Start the shop")?;
    write(
        &dir,
        "src/index.html",
        "<!doctype html>\n<title>Acme Shop</title>\n<link rel=\"stylesheet\" href=\"shop.css\">\n<h1>Acme Shop</h1>\n<main id=\"grid\"></main>\n<script type=\"module\" src=\"main.js\"></script>\n",
    )?;
    write(
        &dir,
        "src/shop.css",
        "body { font-family: system-ui; margin: 2rem; }\n#grid { display: grid; gap: 1rem; }\n",
    )?;
    write(
        &dir,
        "src/catalog.js",
        "export function render(products, grid) {\n  grid.replaceChildren(...products.map(card));\n}\n\nexport function card(p) {\n  const el = document.createElement('article');\n  el.textContent = `${p.name} · ${p.price}`;\n  return el;\n}\n",
    )?;
    write(
        &dir,
        "src/main.js",
        "import { render } from './catalog.js';\n\nrender([], document.getElementById('grid'));\n",
    )?;
    write(&dir, "src/cart.js", "export const cart = [];\n")?;
    write(&dir, "src/checkout/index.js", "export function checkout(cart) {\n  return { items: cart.length };\n}\n")?;
    pdt_git::commit_all(&dir, "Catalog, cart and checkout")?;
    pdt_git::ensure_excluded(&dir, &format!("/{WORKTREES_DIR}/"))?;
    // The path git reports (resolved, e.g. /private/var on macOS), as adding the project in the UI would store it.
    let root = pdt_git::resolve_repo(&dir)?.root;
    Ok(SampleRepo { root })
}

/// A worktree per started ticket, each with one commit of the agent's on its branch.
fn make_branches(repo: &Path, branches: &[(String, Vec<(&str, &str)>)]) -> anyhow::Result<Vec<BranchHead>> {
    let base = pdt_git::rev_parse(repo, "refs/heads/main").context("the sample repository's main")?;
    let mut out = Vec::new();
    for (branch, files) in branches {
        let path = worktree_path(repo, branch);
        let start = pdt_git::add_worktree(repo, &path, branch, &base)?;
        for (file, text) in files {
            write(&path, file, text)?;
        }
        let head = pdt_git::commit_all(&path, &format!("Change {}", files[0].0))?;
        out.push(BranchHead { path, base: start, head });
    }
    Ok(out)
}

fn write(dir: &Path, file: &str, text: &str) -> anyhow::Result<()> {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::pr::PrState;
    use pdt_core::ticket::TicketQuery;

    #[tokio::test]
    async fn seeds_a_board_with_every_stage_once() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let now = 40 * DAY;
        assert!(seed_website(&store, tmp.path(), now).await.unwrap());

        let projects = store.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        let p = &projects[0];
        assert_eq!((p.name.as_str(), p.key.as_str()), (PROJECT_NAME, PROJECT_KEY));
        let config = pdt_git::branch_file(Path::new(&p.repo_path), "main", ".pdt/project.toml").unwrap().unwrap();
        assert!(config.contains("forge = \"local\""));

        let q = TicketQuery { project: Some(p.id.clone()), ..Default::default() }.validate().unwrap();
        let tickets = store.query_tickets(&q).await.unwrap();
        assert_eq!(tickets.total as usize, items().len());
        let stages: Vec<_> = tickets.items.iter().filter_map(|t| t.pipeline_stage).collect();
        for s in
            [PipelineStage::Code, PipelineStage::Review, PipelineStage::Qa, PipelineStage::Accept, PipelineStage::Merge]
        {
            assert!(stages.contains(&s), "a ticket in {s:?}");
        }
        for s in [TicketStage::Todo, TicketStage::Ready, TicketStage::Draft, TicketStage::Done] {
            assert!(tickets.items.iter().any(|t| t.stage == s), "a ticket in {s:?}");
        }
        assert_eq!(tickets.items.iter().filter(|t| t.parent.is_some()).count(), 3, "Checkout v2 has three parts");

        // Every started ticket has an open change request whose CI ran on its branch's head.
        let active = store.list_active_attempts().await.unwrap();
        assert_eq!(active.len(), stages.len());
        for a in &active {
            let change = store.local_change_for_branch(&p.id, &a.attempt.branch).await.unwrap().unwrap();
            assert_eq!(change.state, PrState::Open);
            let head = pdt_git::rev_parse(Path::new(&p.repo_path), &format!("refs/heads/{}", a.attempt.branch));
            assert_eq!(head.as_deref(), Some(change.head_sha.as_str()));
            let runs = store.local_ci_runs(change.id).await.unwrap();
            assert!(runs.iter().all(|r| r.state != CiState::Running && r.head_sha == change.head_sha));
            assert_eq!(runs.len(), 1);
        }
        assert_eq!(store.open_inbox_items(None).await.unwrap().len(), 2, "the Accept gate and a question");

        let sessions = store.list_sessions(&p.id).await.unwrap();
        assert_eq!(sessions.len(), 4);
        assert!(sessions.iter().all(|s| s.status == SessionStatus::Finished));
        let conv = sessions.iter().find(|s| s.mode == SessionMode::Conversation).unwrap();
        assert!(store.list_session_events(&conv.id, 0, 100).await.unwrap().len() > 10);
        assert!(store.list_active_runs().await.unwrap().is_empty());
        // The cart branch's changes carry the user's comment on the line it names.
        let cart = active.iter().find(|a| a.attempt.branch.contains("save-the-cart")).unwrap();
        let wt = store.get_worktree(cart.attempt.worktree_id.as_deref().unwrap()).await.unwrap().unwrap();
        let comments = store.list_review_comments(&wt.id).await.unwrap();
        assert_eq!(comments.len(), 1);
        let text = std::fs::read_to_string(Path::new(&wt.path).join(&comments[0].path)).unwrap();
        assert_eq!(text.lines().nth(comments[0].line as usize - 1), Some(REVIEWED_LINE));
        // A PM conversation in the host project, which lists no project.
        let pm = store.list_pm_sessions().await.unwrap();
        assert_eq!(pm.len(), 1);
        assert!(store.list_session_events(&pm[0].session_id, 0, 100).await.unwrap().len() > 3);
        let workflows = store.list_workflows(None).await.unwrap();
        assert!(workflows.iter().any(|w| w.name == "Nightly dependency updates" && w.enabled));
        // A database with projects is left alone.
        assert!(!seed_website(&store, tmp.path(), now).await.unwrap());
    }
}
