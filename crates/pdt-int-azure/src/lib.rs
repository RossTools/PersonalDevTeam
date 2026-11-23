//! Azure (PDT-638): virtual machines as cloud instances. The `azure` connection holds a service principal (its tenant,
//! client and subscription in the config, the client secret as its secret); the owner signs in with the client
//! credentials grant and starts, deallocates and reads a VM through Azure Resource Manager. A VM is named
//! `resource-group/vm-name`. pdtd's `azure` feature.

use pdt_api::sdk::Integration;
use pdt_api::sdk::cloud::{CloudProvider, CloudTarget, Power};
use pdt_core::integration::{ConfigField, ConnectionKind};
use pdt_forge::transport::{Request, Transport, encode, form};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const AZURE: &str = "azure";
const LOGIN: &str = "https://login.microsoftonline.com";
const ARM: &str = "https://management.azure.com";
/// The Compute API version PDT speaks.
const API_VERSION: &str = "2024-07-01";
/// A token is used until this long before it expires.
const TOKEN_MARGIN_MS: i64 = 5 * 60 * 1000;

pub struct Azure {
    provider: Arc<Arm>,
}

impl Azure {
    /// Reaching Azure through `transport`.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { provider: Arc::new(Arm::new(transport)) }
    }
}

impl Integration for Azure {
    fn id(&self) -> &'static str {
        AZURE
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        let id = |key: &str, label: &str| ConfigField {
            placeholder: Some("00000000-0000-0000-0000-000000000000".into()),
            ..ConfigField::new(key, label)
        };
        vec![ConnectionKind {
            id: AZURE.into(),
            label: "Azure subscription".into(),
            plural: "Azure subscriptions".into(),
            setup: "A service principal that may start, deallocate and read your VMs, for cloud instances.".into(),
            description: "Azure VMs PDT starts for work and deallocates when idle (cloud instances).".into(),
            secret_label: "Client secret".into(),
            secret_hint: "An app registration's client secret. Give the app a role on the VMs' resource group that \
                          allows only Microsoft.Compute/virtualMachines/start, deallocate and read (instanceView)."
                .into(),
            fields: vec![
                id("tenantId", "Tenant (directory) ID"),
                id("clientId", "Client (application) ID"),
                id("subscriptionId", "Subscription ID"),
            ],
            ..Default::default()
        }]
    }

    fn check_connection_config(&self, _kind: &str, config: &Value) -> Result<(), String> {
        settings(config).map(|_| ())
    }

    fn cloud_provider(&self) -> Option<Arc<dyn CloudProvider>> {
        Some(self.provider.clone())
    }
}

/// An `azure` connection's tenant, client and subscription ids.
struct Settings {
    tenant: String,
    client: String,
    subscription: String,
}

fn settings(config: &Value) -> Result<Settings, String> {
    let id = |k: &str, what: &str| {
        let v = config.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if v.is_empty() || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.') {
            return Err(format!("Enter the {what}."));
        }
        Ok(v.to_string())
    };
    Ok(Settings {
        tenant: id("tenantId", "tenant ID")?,
        client: id("clientId", "client ID")?,
        subscription: id("subscriptionId", "subscription ID")?,
    })
}

/// `resource-group/vm-name`.
fn parse_machine(machine: &str) -> Result<(&str, &str), String> {
    let bad = || "An Azure VM is named resource-group/vm-name.".to_string();
    let (group, vm) = machine.split_once('/').ok_or_else(bad)?;
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.()".contains(&b));
    if !ok(group) || !ok(vm) {
        return Err(bad());
    }
    Ok((group, vm))
}

/// A service principal: its tenant, client and secret.
type Principal = (String, String, String);

/// Azure Resource Manager, with the tokens it signed in for.
pub struct Arm {
    transport: Arc<dyn Transport>,
    /// Tokens by tenant, client and secret: the token and when it expires (ms).
    tokens: Mutex<HashMap<Principal, (String, i64)>>,
}

impl Arm {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { transport, tokens: Mutex::default() }
    }

    /// A token for ARM (signing in when there is none still good).
    fn token(&self, s: &Settings, secret: &str) -> Result<String, String> {
        let now = pdt_api::sdk::now_ms();
        let key = (s.tenant.clone(), s.client.clone(), secret.to_string());
        if let Some((token, until)) = self.tokens.lock().unwrap_or_else(|e| e.into_inner()).get(&key)
            && now < until - TOKEN_MARGIN_MS
        {
            return Ok(token.clone());
        }
        let body = form(&[
            ("grant_type", "client_credentials"),
            ("client_id", &s.client),
            ("client_secret", secret),
            ("scope", "https://management.azure.com/.default"),
        ]);
        let req = Request {
            method: "POST",
            url: format!("{LOGIN}/{}/oauth2/v2.0/token", encode(&s.tenant)),
            headers: vec![("content-type", "application/x-www-form-urlencoded".into())],
            body: Some(body),
        };
        let res = self.transport.send(&req).map_err(|e| format!("Azure sign-in can't be reached: {e}"))?;
        let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
        if !(200..300).contains(&res.status) {
            let said = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("no reason given");
            // The first line: Entra adds trace and correlation ids after it.
            return Err(format!("Azure refused the sign-in ({}): {}", res.status, said.lines().next().unwrap_or("")));
        }
        let token = v["access_token"].as_str().ok_or("Azure's sign-in gave no token")?.to_string();
        let until = now + v["expires_in"].as_i64().unwrap_or(3600) * 1000;
        self.tokens.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (token.clone(), until));
        Ok(token)
    }

    /// One ARM call on the VM (`what`: `instanceView`, `start`, `deallocate`): the answer's body.
    fn call(&self, t: &CloudTarget, method: &'static str, what: &str) -> Result<String, String> {
        let s = settings(&t.connection.config)?;
        let (group, vm) = parse_machine(&t.machine)?;
        let token = self.token(&s, &t.secret)?;
        let url = format!(
            "{ARM}/subscriptions/{}/resourceGroups/{}/providers/Microsoft.Compute/virtualMachines/{}/{what}\
             ?api-version={API_VERSION}",
            encode(&s.subscription),
            encode(group),
            encode(vm)
        );
        let body = (method == "POST").then(String::new);
        let req = Request { method, url, headers: vec![("authorization", format!("Bearer {token}"))], body };
        let res = self.transport.send(&req).map_err(|e| format!("Azure can't be reached: {e}"))?;
        if (200..300).contains(&res.status) {
            return Ok(res.body);
        }
        let v: Value = serde_json::from_str(&res.body).unwrap_or(Value::Null);
        let said = match (v["error"]["code"].as_str(), v["error"]["message"].as_str()) {
            (Some(code), Some(msg)) => format!("{code}: {msg}"),
            _ => res.body.chars().take(200).collect(),
        };
        Err(format!("Azure answered {} to {what}: {said}", res.status))
    }
}

/// The power state in a VM's instance view (`PowerState/deallocated`). A VM stopped but not deallocated (shut down
/// from inside) counts as stopped: Start brings it back, but it is billed until it is deallocated.
fn power_of(body: &str) -> Result<Power, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| "Azure's instance view isn't JSON".to_string())?;
    let code = v["statuses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["code"].as_str())
        .find_map(|c| c.strip_prefix("PowerState/"));
    match code {
        Some("running") => Ok(Power::Running),
        Some("starting") => Ok(Power::Starting),
        Some("stopping" | "deallocating") => Ok(Power::Stopping),
        Some("stopped" | "deallocated") => Ok(Power::Stopped),
        _ => Ok(Power::Unknown),
    }
}

impl CloudProvider for Arm {
    fn kind(&self) -> &'static str {
        AZURE
    }
    fn machine_hint(&self) -> &'static str {
        "resource-group/vm-name"
    }
    fn check_machine(&self, machine: &str) -> Result<(), String> {
        parse_machine(machine).map(|_| ())
    }
    fn power(&self, t: &CloudTarget) -> Result<Power, String> {
        power_of(&self.call(t, "GET", "instanceView")?)
    }
    fn start(&self, t: &CloudTarget) -> Result<(), String> {
        self.call(t, "POST", "start").map(|_| ())
    }
    fn stop(&self, t: &CloudTarget) -> Result<(), String> {
        self.call(t, "POST", "deallocate").map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::api::Connection;
    use pdt_forge::transport::Response;
    use pdt_forge::transport::testing::Recorded;
    use serde_json::json;

    const TOKEN_URL: &str = "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token";
    const VM: &str = "https://management.azure.com/subscriptions/sub-1/resourceGroups/pdt/providers/\
                      Microsoft.Compute/virtualMachines/win-cloud";

    fn target() -> CloudTarget {
        CloudTarget {
            connection: Connection {
                id: "c".into(),
                name: "azure".into(),
                kind: AZURE.into(),
                url: String::new(),
                config: json!({"tenantId": "tenant-1", "clientId": "client-1", "subscriptionId": "sub-1"}),
                has_secret: true,
                created_at: 0,
                updated_at: 0,
                last_error: None,
                last_error_at: None,
            },
            secret: "s3cret".into(),
            machine: "pdt/win-cloud".into(),
        }
    }

    /// An instance view as ARM answers it (compact JSON, as it comes).
    fn view(power: &str) -> String {
        format!(
            r#"{{"statuses":[{{"code":"ProvisioningState/succeeded","level":"Info"}},{{"code":"PowerState/{power}","level":"Info"}}]}}"#
        )
    }

    #[test]
    fn it_signs_in_once_and_reads_starts_and_deallocates_a_vm() {
        let net = Arc::new(Recorded::new());
        net.ok("POST", TOKEN_URL, r#"{"token_type":"Bearer","expires_in":3599,"access_token":"tok"}"#)
            .ok("GET", &format!("{VM}/instanceView?api-version=2024-07-01"), &view("deallocated"))
            .on("POST", &format!("{VM}/start?api-version=2024-07-01"), Response::new(202, ""))
            .on("POST", &format!("{VM}/deallocate?api-version=2024-07-01"), Response::new(202, ""));
        let arm = Arm::new(net.clone());
        let t = target();
        assert_eq!(arm.power(&t), Ok(Power::Stopped));
        arm.start(&t).unwrap();
        arm.stop(&t).unwrap();
        let sent = net.sent();
        assert_eq!(sent.len(), 4, "{:?}", net.urls());
        assert_eq!(
            sent[0].body.as_deref(),
            Some(
                "grant_type=client_credentials&client_id=client-1&client_secret=s3cret&scope=https%3A%2F%2F\
                 management.azure.com%2F.default"
            )
        );
        for r in &sent[1..] {
            assert!(r.headers.contains(&("authorization", "Bearer tok".into())), "{r:?}");
        }
    }

    #[test]
    fn refusals_say_why() {
        let net = Arc::new(Recorded::new());
        let refused = r#"{"error":"invalid_client","error_description":"AADSTS7000215: Invalid client secret provided.\r\nTrace ID: x"}"#;
        net.on("POST", TOKEN_URL, Response::new(401, refused));
        let arm = Arm::new(net.clone());
        assert_eq!(
            arm.power(&target()),
            Err("Azure refused the sign-in (401): AADSTS7000215: Invalid client secret provided.".into())
        );

        let net = Arc::new(Recorded::new());
        let missing = r#"{"error":{"code":"ResourceNotFound","message":"The Resource 'win-cloud' was not found."}}"#;
        net.ok("POST", TOKEN_URL, r#"{"expires_in":3599,"access_token":"tok"}"#).on(
            "POST",
            &format!("{VM}/start"),
            Response::new(404, missing),
        );
        let arm = Arm::new(net);
        assert_eq!(
            arm.start(&target()),
            Err("Azure answered 404 to start: ResourceNotFound: The Resource 'win-cloud' was not found.".into())
        );
    }

    #[test]
    fn power_states_and_machines() {
        assert_eq!(power_of(&view("running")), Ok(Power::Running));
        assert_eq!(power_of(&view("starting")), Ok(Power::Starting));
        assert_eq!(power_of(&view("deallocating")), Ok(Power::Stopping));
        assert_eq!(power_of(&view("stopped")), Ok(Power::Stopped));
        assert_eq!(power_of(r#"{"statuses":[]}"#), Ok(Power::Unknown));
        assert!(parse_machine("pdt/win-cloud").is_ok());
        assert!(parse_machine("win-cloud").is_err());
        assert!(parse_machine("pdt/win cloud").is_err());
        assert!(settings(&json!({"tenantId": "t", "clientId": "c"})).is_err());
    }
}
