//! AWS (PDT-638): EC2 instances as cloud instances. The `aws` connection holds an access key (its id and region in the
//! config, the secret access key as its secret); the owner starts, stops and reads an instance through the EC2 Query
//! API, signed with Signature Version 4 ([`sigv4`]). pdtd's `aws` feature.

pub mod sigv4;

use pdt_api::sdk::Integration;
use pdt_api::sdk::cloud::{CloudProvider, CloudTarget, Power};
use pdt_core::integration::{ConfigField, ConnectionKind};
use pdt_forge::transport::{Request, Transport, form};
use serde_json::Value;
use std::sync::Arc;

pub const AWS: &str = "aws";
/// The EC2 API version PDT speaks.
const VERSION: &str = "2016-11-15";

pub struct Aws {
    provider: Arc<Ec2>,
}

impl Aws {
    /// Reaching EC2 through `transport`.
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self { provider: Arc::new(Ec2 { transport, now: None }) }
    }
}

impl Integration for Aws {
    fn id(&self) -> &'static str {
        AWS
    }

    fn connection_kinds(&self) -> Vec<ConnectionKind> {
        vec![ConnectionKind {
            id: AWS.into(),
            label: "AWS account".into(),
            plural: "AWS accounts".into(),
            setup: "An access key that may start, stop and describe your EC2 instances, for cloud instances.".into(),
            description: "EC2 instances PDT starts for work and stops when idle (cloud instances).".into(),
            url: true,
            url_optional: true,
            url_placeholder: Some("https://ec2.us-east-1.amazonaws.com".into()),
            url_hint: Some("Leave it empty for the region's EC2 endpoint.".into()),
            secret_label: "Secret access key".into(),
            secret_hint: "An IAM user's access key, allowed only ec2:StartInstances, ec2:StopInstances and \
                          ec2:DescribeInstances (scope it to the instances' ARNs)."
                .into(),
            fields: vec![
                ConfigField { placeholder: Some("AKIA…".into()), ..ConfigField::new("accessKeyId", "Access key ID") },
                ConfigField { placeholder: Some("us-east-1".into()), ..ConfigField::new("region", "Region") },
            ],
            summary: Some("{region}".into()),
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

/// An `aws` connection's access key id and region.
fn settings(config: &Value) -> Result<(String, String), String> {
    let text = |k: &str| config.get(k).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
    let (key, region) = (text("accessKeyId"), text("region"));
    if key.is_empty() {
        return Err("Enter the access key ID.".into());
    }
    if region.is_empty() || !region.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err("Enter the region the instances are in (us-east-1).".into());
    }
    Ok((key, region))
}

/// The EC2 Query API.
pub struct Ec2 {
    transport: Arc<dyn Transport>,
    /// The signing time (ms) in tests; now when None.
    now: Option<i64>,
}

impl Ec2 {
    /// For tests: EC2 through `transport`, signing as at `now` (ms).
    pub fn at(transport: Arc<dyn Transport>, now: i64) -> Self {
        Self { transport, now: Some(now) }
    }

    /// One signed call of `action` on instance `t.machine`: the answer's body, or the error EC2 gave.
    fn call(&self, t: &CloudTarget, action: &str) -> Result<String, String> {
        let (key, region) = settings(&t.connection.config)?;
        let endpoint = match t.connection.url.trim().trim_end_matches('/') {
            "" => format!("https://ec2.{region}.amazonaws.com"),
            url => url.to_string(),
        };
        let host = endpoint.split("://").nth(1).unwrap_or(&endpoint).to_string();
        let body = form(&[("Action", action), ("InstanceId.1", &t.machine), ("Version", VERSION)]);
        let now = self.now.unwrap_or_else(pdt_api::sdk::now_ms);
        let signed = sigv4::sign(&sigv4::Signing {
            method: "POST",
            host: &host,
            path: "/",
            query: "",
            content_type: sigv4::FORM,
            body: &body,
            key_id: &key,
            secret: &t.secret,
            region: &region,
            service: "ec2",
            now,
        });
        let req = Request {
            method: "POST",
            url: format!("{endpoint}/"),
            headers: vec![
                ("content-type", sigv4::FORM.into()),
                ("x-amz-date", signed.amz_date),
                ("authorization", signed.authorization),
            ],
            body: Some(body),
        };
        let res = self.transport.send(&req).map_err(|e| format!("EC2 can't be reached: {e}"))?;
        if (200..300).contains(&res.status) {
            return Ok(res.body);
        }
        let said = match (tag(&res.body, "Code"), tag(&res.body, "Message")) {
            (Some(code), Some(msg)) => format!("{code}: {msg}"),
            (Some(code), None) => code.to_string(),
            _ => res.body.chars().take(200).collect(),
        };
        Err(format!("EC2 answered {} to {action}: {said}", res.status))
    }
}

/// The text of the first `<name>…</name>` in `xml`.
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim())
}

/// The power state in a DescribeInstances answer.
fn power_of(xml: &str) -> Result<Power, String> {
    let state = xml.find("<instanceState>").map(|i| &xml[i..]).and_then(|s| tag(s, "name"));
    match state {
        Some("pending") => Ok(Power::Starting),
        Some("running") => Ok(Power::Running),
        Some("stopping" | "shutting-down") => Ok(Power::Stopping),
        Some("stopped") => Ok(Power::Stopped),
        Some("terminated") => Err("the instance was terminated".into()),
        Some(other) => Err(format!("EC2 says the instance is {other}")),
        None => Err("EC2 didn't list the instance".into()),
    }
}

impl CloudProvider for Ec2 {
    fn kind(&self) -> &'static str {
        AWS
    }
    fn machine_hint(&self) -> &'static str {
        "i-0123456789abcdef0"
    }
    fn check_machine(&self, machine: &str) -> Result<(), String> {
        let hex = machine.strip_prefix("i-").unwrap_or_default();
        if hex.len() < 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("An EC2 instance id looks like i-0123456789abcdef0.".into());
        }
        Ok(())
    }
    fn power(&self, t: &CloudTarget) -> Result<Power, String> {
        power_of(&self.call(t, "DescribeInstances")?)
    }
    fn start(&self, t: &CloudTarget) -> Result<(), String> {
        self.call(t, "StartInstances").map(|_| ())
    }
    fn stop(&self, t: &CloudTarget) -> Result<(), String> {
        self.call(t, "StopInstances").map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdt_core::workflow::api::Connection;
    use pdt_forge::transport::Response;
    use pdt_forge::transport::testing::Recorded;
    use serde_json::json;

    /// 2026-11-23T12:00:00Z.
    const NOW: i64 = 1_795_435_200_000;

    fn target(url: &str) -> CloudTarget {
        CloudTarget {
            connection: Connection {
                id: "c".into(),
                name: "aws".into(),
                kind: AWS.into(),
                url: url.into(),
                config: json!({"accessKeyId": "AKIDEXAMPLE", "region": "us-east-1"}),
                has_secret: true,
                created_at: 0,
                updated_at: 0,
                last_error: None,
                last_error_at: None,
            },
            secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            machine: "i-0123456789abcdef0".into(),
        }
    }

    const DESCRIBED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<DescribeInstancesResponse xmlns="http://ec2.amazonaws.com/doc/2016-11-15/">
  <reservationSet><item><instancesSet><item>
    <instanceId>i-0123456789abcdef0</instanceId>
    <instanceState><code>80</code><name>stopped</name></instanceState>
    <tagSet><item><key>name</key><value>win-cloud</value></item></tagSet>
  </item></instancesSet></item></reservationSet>
</DescribeInstancesResponse>"#;

    #[test]
    fn it_reads_starts_and_stops_an_instance_with_signed_calls() {
        let net = Arc::new(Recorded::new());
        net.ok("POST", "https://ec2.us-east-1.amazonaws.com/", DESCRIBED)
            .ok("POST", "https://ec2.us-east-1.amazonaws.com/", "<StartInstancesResponse/>")
            .ok("POST", "https://ec2.us-east-1.amazonaws.com/", "<StopInstancesResponse/>");
        let ec2 = Ec2::at(net.clone(), NOW);
        let t = target("");
        assert_eq!(ec2.power(&t), Ok(Power::Stopped));
        ec2.start(&t).unwrap();
        ec2.stop(&t).unwrap();
        let sent = net.sent();
        let bodies: Vec<&str> = sent.iter().map(|r| r.body.as_deref().unwrap()).collect();
        assert_eq!(
            bodies,
            [
                "Action=DescribeInstances&InstanceId.1=i-0123456789abcdef0&Version=2016-11-15",
                "Action=StartInstances&InstanceId.1=i-0123456789abcdef0&Version=2016-11-15",
                "Action=StopInstances&InstanceId.1=i-0123456789abcdef0&Version=2016-11-15",
            ]
        );
        let header = |r: &Request, k: &str| r.headers.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(header(&sent[0], "x-amz-date"), "20261123T120000Z");
        let auth = header(&sent[0], "authorization");
        assert!(
            auth.starts_with(
                "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20261123/us-east-1/ec2/aws4_request, \
                 SignedHeaders=content-type;host;x-amz-date, Signature="
            ),
            "{auth}"
        );
        assert!(!auth.contains(&t.secret), "never the secret itself");
    }

    #[test]
    fn errors_say_what_ec2_said_and_an_endpoint_can_be_set() {
        let net = Arc::new(Recorded::new());
        let refused = "<Response><Errors><Error><Code>UnauthorizedOperation</Code><Message>You are not authorized \
                       to perform this operation.</Message></Error></Errors></Response>";
        net.on("POST", "http://127.0.0.1:4566/", Response::new(403, refused));
        let ec2 = Ec2::at(net.clone(), NOW);
        assert_eq!(
            ec2.start(&target("http://127.0.0.1:4566/")),
            Err("EC2 answered 403 to StartInstances: UnauthorizedOperation: You are not authorized to perform this \
                 operation."
                .into())
        );
    }

    #[test]
    fn states_map_to_power() {
        let with = |name: &str| DESCRIBED.replace("<name>stopped</name>", &format!("<name>{name}</name>"));
        assert_eq!(power_of(&with("pending")), Ok(Power::Starting));
        assert_eq!(power_of(&with("running")), Ok(Power::Running));
        assert_eq!(power_of(&with("stopping")), Ok(Power::Stopping));
        assert!(power_of(&with("terminated")).is_err());
        assert!(power_of("<DescribeInstancesResponse/>").is_err());
    }

    #[test]
    fn settings_and_machines_are_checked() {
        assert!(settings(&json!({"accessKeyId": "AKIA", "region": "eu-west-1"})).is_ok());
        assert!(settings(&json!({"accessKeyId": "AKIA", "region": "EU West"})).is_err());
        assert!(settings(&json!({"region": "eu-west-1"})).is_err());
        let ec2 = Ec2::at(Arc::new(Recorded::new()), NOW);
        assert!(ec2.check_machine("i-0123456789abcdef0").is_ok());
        assert!(ec2.check_machine("win-cloud").is_err());
    }
}
