//! Cloud instances (PDT-638): win-box is a VM at a fake cloud provider. A ticket placed on it while it is stopped
//! waits ("starting win-box…") while the owner starts it, starts on it once its PDT answers, and the VM is stopped once
//! it has been idle for its minutes, its running time metered. Start and Stop by hand, and the setup's checks.

use crate::common::{self, call, call_as_ui};
use crate::remote_attempts::{get_ticket, paired_set};
use axum::http::StatusCode;
use pdt_api::sdk::Integration;
use pdt_api::sdk::cloud::{CloudProvider, CloudTarget, Power};
use pdt_core::integration::ConnectionKind;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

/// A cloud provider that remembers what it was asked, with its VM's power set by the test.
struct FakeCloud {
    power: Mutex<Power>,
    calls: Mutex<Vec<String>>,
}

impl FakeCloud {
    fn set(&self, p: Power) {
        *self.power.lock().unwrap() = p;
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl CloudProvider for FakeCloud {
    fn kind(&self) -> &'static str {
        "fake_cloud"
    }
    fn machine_hint(&self) -> &'static str {
        "vm-1"
    }
    fn check_machine(&self, machine: &str) -> Result<(), String> {
        if machine.contains(' ') { Err("no spaces".into()) } else { Ok(()) }
    }
    fn power(&self, t: &CloudTarget) -> Result<Power, String> {
        assert_eq!(t.secret, "key");
        Ok(*self.power.lock().unwrap())
    }
    fn start(&self, t: &CloudTarget) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("start {}", t.machine));
        self.set(Power::Starting);
        Ok(())
    }
    fn stop(&self, t: &CloudTarget) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("stop {}", t.machine));
        self.set(Power::Stopping);
        Ok(())
    }
}

struct Plugin(Arc<FakeCloud>);

impl Integration for Plugin {
    fn id(&self) -> &'static str {
        "fake_cloud"
    }
    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind { id: "fake_cloud".into(), label: "Fake cloud".into(), ..Default::default() }]
    }
    fn cloud_provider(&self) -> Option<Arc<dyn CloudProvider>> {
        Some(self.0.clone())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

#[tokio::test]
async fn a_stopped_cloud_instance_starts_for_work_placed_on_it_and_stops_once_idle() {
    let cloud = Arc::new(FakeCloud { power: Mutex::new(Power::Running), calls: Mutex::default() });
    let plugin = Plugin(cloud.clone());
    let p = paired_set(None, None, move |st| {
        st.integrations = Arc::new(pdt_api::Integrations::new(vec![Arc::new(plugin)]));
        st.integrations.enable(&["fake_cloud".into()]);
    })
    .await;
    let owner = &p.owner;
    let base = format!("/api/projects/{}/remote-instances", p.project);
    let (_, v) = call(&owner.app, "GET", &base, None).await;
    assert_eq!(v["cloudProviders"], json!([{"kind": "fake_cloud", "machineHint": "vm-1"}]), "{v}");
    let link = v["instances"][0]["linkId"].as_str().unwrap().to_string();
    assert!(v["instances"][0]["cloud"].is_null());

    // The provider's credentials are a connection of its kind.
    let body = json!({"name": "my cloud", "kind": "fake_cloud", "url": "", "config": {}});
    let (s, c) = call(&owner.app, "POST", "/api/connections", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{c}");
    let secret = format!("/api/connections/{}/secret", c["id"].as_str().unwrap());
    let (s, v) = call(&owner.app, "PUT", &secret, Some(json!({"secret": "key"}))).await;
    assert!(s.is_success(), "{v}");
    let http = json!({"name": "svc", "kind": "http", "url": "http://127.0.0.1:9", "config": {}});
    assert_eq!(call(&owner.app, "POST", "/api/connections", Some(http)).await.0, StatusCode::CREATED);

    let cloud_url = format!("{base}/{link}/cloud");
    let setup = |connection: &str, machine: &str| json!({"connection": connection, "machine": machine, "idleMinutes": 30, "hourlyRate": 0.5});
    for (bad, why) in [
        (setup("nope", "vm-1"), "no connection is called nope"),
        (setup("svc", "vm-1"), "svc is a http connection, not a cloud provider's"),
        (setup("my cloud", "vm 1"), "no spaces"),
    ] {
        let (s, v) = call(&owner.app, "PUT", &cloud_url, Some(bad)).await;
        assert_eq!((s, v["message"].as_str().unwrap()), (StatusCode::BAD_REQUEST, why));
    }
    let (s, v) = call(&owner.app, "PUT", &cloud_url, Some(setup("my cloud", " vm-1 "))).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["instances"][0]["cloud"]["setup"]["machine"], "vm-1");

    // Auto-start hands each start over to a workflow (no agent session).
    let doc = json!({"name": "Takes over", "nodes": [
        {"id": "started", "kind": "attempt.started", "version": 1},
        {"id": "end", "kind": "end", "version": 1}],
        "edges": [{"from": "started", "to": "end"}]});
    let body = json!({"doc": doc, "scope": "project", "projectId": p.project, "enabled": true});
    let (s, v) = call(&owner.app, "POST", "/api/workflows", Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let settings = json!({"agentCredentials": null, "runOverrides": null, "autoStart": "todo", "startDelaySeconds": 0});
    let (s, v) = call(&owner.app, "PUT", &format!("/api/projects/{}/settings", p.project), Some(settings)).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // The VM is stopped: win-box went quiet a while ago.
    cloud.set(Power::Stopped);
    owner.state.link.set_offline(true);
    let facts = pdt_core::link::InstanceFacts::default();
    let (_, v) = call(&owner.app, "GET", &base, None).await;
    let quiet = pdt_store::Heard::Answer {
        name: "win-box",
        version: v["instances"][0]["version"].as_str().unwrap(),
        protocol: pdt_core::link::PROTOCOL,
        facts: &facts,
        paused: false,
        secrets: &[],
        cap: None,
        at: now_ms() - 60 * 60 * 1000,
    };
    owner.state.store.record_heartbeat(&link, quiet).await.unwrap();
    pdt_api::cloud::check_all(&owner.state).await;
    assert!(cloud.calls().is_empty(), "nothing waits for it: it stays stopped");

    // A windows ticket waits while the owner starts the VM.
    let body = json!({"title": "Client", "labels": ["windows"], "stage": "todo"});
    let (s, t) = call_as_ui(&owner.app, "POST", &format!("/api/projects/{}/tickets", p.project), Some(body)).await;
    assert_eq!(s, StatusCode::CREATED, "{t}");
    let ticket = t["id"].as_str().unwrap().to_string();
    common::pick_up(&owner.state).await;
    let t = get_ticket(&p, &ticket).await;
    assert_eq!((t["stage"].as_str(), t["pickupHold"].as_str()), (Some("todo"), Some("starting win-box…")));
    pdt_api::cloud::check_all(&owner.state).await;
    assert_eq!(cloud.calls(), ["start vm-1"]);
    let (_, v) = call(&owner.app, "GET", &base, None).await;
    assert_eq!(v["instances"][0]["cloud"]["starting"], true, "{v}");
    pdt_api::cloud::check_all(&owner.state).await;
    assert_eq!(cloud.calls(), ["start vm-1"], "asked once");

    // It boots and its PDT answers: the ticket starts on it.
    cloud.set(Power::Running);
    owner.state.link.set_offline(false);
    pdt_api::link::heartbeat_all(&owner.state).await;
    pdt_api::cloud::check_all(&owner.state).await;
    common::pick_up(&owner.state).await;
    assert_eq!(get_ticket(&p, &ticket).await["stage"], "in_progress");
    let (_, v) = call(&owner.app, "GET", &base, None).await;
    let c = &v["instances"][0]["cloud"];
    assert_eq!((&c["starting"], &c["state"]["power"]), (&json!(false), &json!("running")), "{v}");

    // Nothing live on it: once idle for its 30 minutes, it is stopped, and the meter counted its running time.
    let up = now_ms();
    pdt_api::cloud::check_at(&owner.state, up + 60_000).await;
    pdt_api::cloud::check_at(&owner.state, up + 29 * 60_000).await;
    assert_eq!(cloud.calls(), ["start vm-1"]);
    pdt_api::cloud::check_at(&owner.state, up + 31 * 60_000).await;
    assert_eq!(cloud.calls(), ["start vm-1", "stop vm-1"]);
    let (_, v) = call(&owner.app, "GET", &base, None).await;
    let usage = &v["instances"][0]["cloud"]["usage"][0];
    let hours = usage["hours"].as_f64().unwrap();
    assert!(hours > 0.1, "{v}");
    assert!((usage["cost"].as_f64().unwrap() - hours * 0.5).abs() < 1e-6, "{v}");

    // By hand.
    cloud.set(Power::Stopped);
    let (s, v) = call(&owner.app, "POST", &format!("{cloud_url}/start"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["instances"][0]["cloud"]["starting"], true);
    let (s, _) = call(&owner.app, "POST", &format!("{cloud_url}/stop"), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(cloud.calls()[2..], ["start vm-1", "stop vm-1"]);

    // No longer a cloud instance.
    let (s, v) = call(&owner.app, "DELETE", &cloud_url, None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["instances"][0]["cloud"], Value::Null);
}
