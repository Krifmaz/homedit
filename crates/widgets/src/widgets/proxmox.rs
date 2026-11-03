//! Proxmox VE: running guests and node load.
//!
//! Authenticates with an API token in the `Authorization` header
//! (`PVEAPIToken=<user@realm!tokenid>=<secret>`) and reads everything from
//! one call to `/api2/json/cluster/resources`, which works on single nodes
//! and clusters alike. The token needs at least the `PVEAuditor` role.

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

use crate::config::WidgetConfig;
use crate::context::WidgetContext;
use crate::data::{Stat, StatValue, WidgetData};
use crate::error::WidgetError;
use crate::schema::{FieldSpec, FieldType, WidgetSchema};
use crate::widget::Widget;
use crate::widgets::{insecure_tls_field, password_field, username_field};

/// Proxmox VE widget.
#[derive(Debug, Default, Clone, Copy)]
pub struct Proxmox;

/// A Proxmox API envelope: every response wraps its payload in `data`.
#[derive(Debug, Default, Deserialize)]
pub struct Envelope<T> {
    /// The payload.
    #[serde(default)]
    pub data: T,
}

/// One entry of `GET /api2/json/cluster/resources` (only the fields used).
#[derive(Debug, Default, Deserialize)]
pub struct Resource {
    /// `node`, `qemu`, `lxc`, `storage`, `sdn`, ...
    #[serde(default, rename = "type")]
    pub kind: String,
    /// `online`/`offline` for nodes, `running`/`stopped` for guests.
    #[serde(default)]
    pub status: String,
    /// The node this resource lives on.
    #[serde(default)]
    pub node: String,
    /// CPU load from 0 to 1.
    #[serde(default)]
    pub cpu: f64,
    /// Number of CPUs.
    #[serde(default)]
    pub maxcpu: f64,
    /// Memory used in bytes.
    #[serde(default)]
    pub mem: f64,
    /// Memory available in bytes.
    #[serde(default)]
    pub maxmem: f64,
    /// Whether the guest is a template (sent as `0`/`1`).
    #[serde(default, deserialize_with = "truthy")]
    pub template: bool,
}

/// `GET /api2/json/version` payload.
#[derive(Debug, Default, Deserialize)]
pub struct Version {
    /// Version, e.g. `8.2.7`.
    #[serde(default)]
    pub version: String,
}

/// Accepts booleans, numbers (non-zero is true) and `"1"`/`"true"`.
fn truthy<'de, D: Deserializer<'de>>(de: D) -> Result<bool, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => matches!(s.trim(), "1" | "true"),
        _ => false,
    })
}

/// Counts running and total guests of one type.
fn guests<'a>(resources: impl Iterator<Item = &'a Resource> + Clone, kind: &str) -> (u64, u64) {
    let of_kind = resources.filter(|r| r.kind == kind && !r.template);
    let running = of_kind.clone().filter(|r| r.status == "running").count();
    (running as u64, of_kind.count() as u64)
}

/// Parses the cluster resource list into widget data.
///
/// When `node` is given only resources on that node count. Stats: running
/// and total VMs and containers (templates skipped), and CPU and memory use
/// averaged over the online nodes.
pub fn parse_proxmox(resources: &[Resource], node: Option<&str>) -> WidgetData {
    let scoped = resources
        .iter()
        .filter(|r| node.is_none_or(|n| r.node == n));
    let (vms_running, vms_total) = guests(scoped.clone(), "qemu");
    let (lxc_running, lxc_total) = guests(scoped.clone(), "lxc");
    let online: Vec<&Resource> = scoped
        .filter(|r| r.kind == "node" && r.status == "online")
        .collect();
    let avg = |f: &dyn Fn(&Resource) -> f64| {
        if online.is_empty() {
            0.0
        } else {
            online.iter().map(|r| f(r)).sum::<f64>() / online.len() as f64
        }
    };
    let cpu = avg(&|r| r.cpu * 100.0);
    let mem = avg(&|r| {
        if r.maxmem > 0.0 {
            r.mem / r.maxmem * 100.0
        } else {
            0.0
        }
    });
    WidgetData::new(vec![
        Stat::new(
            "vms",
            "VMs",
            StatValue::Text(format!("{vms_running} / {vms_total}")),
        ),
        Stat::new(
            "lxc",
            "LXC",
            StatValue::Text(format!("{lxc_running} / {lxc_total}")),
        ),
        Stat::new("cpu", "CPU", StatValue::Percent(cpu)),
        Stat::new("mem", "Memory", StatValue::Percent(mem)),
    ])
}

/// Builds the `Authorization` header value for an API token.
fn auth_header(cfg: &WidgetConfig) -> Result<String, WidgetError> {
    let token_id = cfg.require_str("username")?;
    let secret = cfg.require_str("password")?;
    Ok(format!("PVEAPIToken={token_id}={secret}"))
}

#[async_trait]
impl Widget for Proxmox {
    fn kind(&self) -> &'static str {
        "proxmox"
    }

    fn schema(&self) -> WidgetSchema {
        WidgetSchema::new(
            "proxmox",
            "Proxmox VE",
            "Running VMs and containers, CPU and memory use.",
        )
        .field(
            username_field(true).help(
                "API token ID, e.g. api@pam!homedit (Datacenter → Permissions → API Tokens).",
            ),
        )
        .field(password_field(true).help("The API token secret."))
        .field(
            FieldSpec::new("node", "Node", FieldType::Text)
                .help("Only count this node. Leave empty for the whole cluster."),
        )
        .field(insecure_tls_field())
    }

    async fn fetch(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<WidgetData, WidgetError> {
        let auth = auth_header(cfg)?;
        let resp: Envelope<Vec<Resource>> = ctx
            .get_json(
                cfg,
                "/api2/json/cluster/resources",
                &[("Authorization", &auth)],
            )
            .await?;
        Ok(parse_proxmox(&resp.data, cfg.str("node")))
    }

    async fn test_connection(
        &self,
        ctx: &WidgetContext,
        cfg: &WidgetConfig,
    ) -> Result<String, WidgetError> {
        let auth = auth_header(cfg)?;
        let resp: Envelope<Version> = ctx
            .get_json(cfg, "/api2/json/version", &[("Authorization", &auth)])
            .await?;
        Ok(format!("Connected to Proxmox VE {}", resp.data.version)
            .trim()
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::fixture;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn resources() -> Vec<Resource> {
        let env: Envelope<Vec<Resource>> =
            serde_json::from_value(fixture("proxmox/cluster_resources.json")).unwrap();
        env.data
    }

    fn text(d: &WidgetData, key: &str) -> String {
        match &d.stat(key).unwrap().value {
            StatValue::Text(t) => t.clone(),
            v => panic!("{key}: {v:?}"),
        }
    }

    fn pct(d: &WidgetData, key: &str) -> f64 {
        match d.stat(key).unwrap().value {
            StatValue::Percent(p) => p,
            ref v => panic!("{key}: {v:?}"),
        }
    }

    #[test]
    fn parses_cluster() {
        let d = parse_proxmox(&resources(), None);
        assert_eq!(text(&d, "vms"), "1 / 2");
        assert_eq!(text(&d, "lxc"), "2 / 3");
        assert!((pct(&d, "cpu") - 21.0).abs() < 1e-9);
        assert!((pct(&d, "mem") - 37.5).abs() < 1e-9);
    }

    #[test]
    fn filters_by_node() {
        let d = parse_proxmox(&resources(), Some("pve2"));
        assert_eq!(text(&d, "vms"), "0 / 1");
        assert_eq!(text(&d, "lxc"), "1 / 2");
        assert!((pct(&d, "cpu") - 30.0).abs() < 1e-9);
        assert!((pct(&d, "mem") - 25.0).abs() < 1e-9);
        let offline = parse_proxmox(&resources(), Some("pve3"));
        assert_eq!(pct(&offline, "cpu"), 0.0);
    }

    #[tokio::test]
    async fn fetch_sends_token_header() {
        let server = MockServer::start().await;
        let auth = "PVEAPIToken=api@pam!homedit=0b0e6c1a-7f1d-4c55-9a43-2f5b1c3d4e5f";
        Mock::given(method("GET"))
            .and(path("/api2/json/cluster/resources"))
            .and(header("authorization", auth))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(fixture("proxmox/cluster_resources.json")),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api2/json/version"))
            .and(header("authorization", auth))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture("proxmox/version.json")))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "api@pam!homedit")
            .with("password", "0b0e6c1a-7f1d-4c55-9a43-2f5b1c3d4e5f");
        let d = Proxmox.fetch(&ctx, &cfg).await.unwrap();
        assert_eq!(text(&d, "vms"), "1 / 2");
        let msg = Proxmox.test_connection(&ctx, &cfg).await.unwrap();
        assert_eq!(msg, "Connected to Proxmox VE 8.2.7");
    }

    #[tokio::test]
    async fn bad_token_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let ctx = WidgetContext::with_defaults().unwrap();
        let cfg = WidgetConfig::default()
            .with("url", server.uri())
            .with("username", "api@pam!homedit")
            .with("password", "wrong");
        let err = Proxmox.fetch(&ctx, &cfg).await.unwrap_err();
        assert!(matches!(err, WidgetError::Auth), "{err:?}");
    }
}
