//! Docker label discovery.
//!
//! Every minute the server lists the containers of each Docker host on the
//! dashboard (`docker.yaml` servers), plus `HOMEDIT_DOCKER_HOST` when set
//! (`unix:///var/run/docker.sock` or `tcp://host:port`, registered as the
//! server `local`), reads their gethomepage-compatible `homepage.*` labels,
//! and adds a card for every labelled container that has none yet.
//!
//! Each container is added once: its `server/container` key is remembered in
//! the setting `discoveredContainers`, so deleting a discovered card in the
//! editor keeps it deleted. Remove the labels (or clear that setting) to
//! change that. The socket is only read; nothing is ever sent to Docker
//! except container list requests.

use std::time::Duration;

use homedit_core::{Dashboard, DockerHost, Group};
use homedit_discovery::{DiscoverOptions, DockerClient, DockerEndpoint, discover_from};
use serde_json::Value;

use crate::AppState;
use crate::status::LiveEvent;

/// Time between discovery rounds.
pub const INTERVAL: Duration = Duration::from_secs(60);
/// Setting that lists containers already added.
pub const SEEN_KEY: &str = "discoveredContainers";

/// Parses `HOMEDIT_DOCKER_HOST` into a Docker host named `local`.
pub fn env_host(value: &str) -> Option<DockerHost> {
    let value = value.trim();
    let mut h = DockerHost {
        name: "local".into(),
        ..DockerHost::default()
    };
    if let Some(path) = value.strip_prefix("unix://") {
        h.socket = Some(path.to_owned());
    } else if let Some(rest) = value.strip_prefix("tcp://") {
        let (host, port) = rest.rsplit_once(':')?;
        h.host = Some(host.to_owned());
        h.port = Some(port.trim_end_matches('/').parse().ok()?);
    } else if value.starts_with('/') {
        h.socket = Some(value.to_owned());
    } else {
        return None;
    }
    Some(h)
}

/// Where to reach a Docker host: its socket, or its TCP address (port 2375 by default).
pub fn endpoint(h: &DockerHost) -> Option<DockerEndpoint> {
    if let Some(s) = &h.socket {
        return Some(DockerEndpoint::Unix(s.into()));
    }
    Some(DockerEndpoint::Tcp {
        host: h.host.clone()?,
        port: h.port.unwrap_or(2375),
    })
}

/// Adds `services` (group name, card, key) to `dash`, skipping keys in the
/// seen list and cards that already point at the same container. Returns
/// how many cards were added.
pub fn merge(dash: &mut Dashboard, found: Vec<(String, homedit_core::Service, String)>) -> usize {
    let mut seen: Vec<String> = dash
        .settings
        .extra
        .get(SEEN_KEY)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let existing: Vec<String> = dash
        .services()
        .filter_map(|s| {
            Some(format!(
                "{}/{}",
                s.server.as_deref().unwrap_or_default(),
                s.container.as_deref()?
            ))
        })
        .collect();
    let mut added = 0;
    for (group, service, key) in found {
        if seen.contains(&key) {
            continue;
        }
        seen.push(key.clone());
        if existing.contains(&key) {
            continue;
        }
        let g = match dash.groups.iter().position(|g| g.name == group) {
            Some(i) => &mut dash.groups[i],
            None => {
                dash.groups.push(Group {
                    name: group,
                    ..Group::default()
                });
                dash.groups.last_mut().expect("just pushed")
            }
        };
        g.services.push(service);
        added += 1;
    }
    dash.settings.extra.insert(
        SEEN_KEY.into(),
        Value::Array(seen.into_iter().map(Value::String).collect()),
    );
    added
}

/// Runs one discovery round. Returns how many cards were added.
pub async fn run_once(state: &AppState, env: Option<&DockerHost>) -> anyhow::Result<usize> {
    let (version, mut dash) = state.db.load_versioned().await?;
    let mut hosts = dash.docker_hosts.clone();
    if let Some(h) = env
        && !hosts.iter().any(|x| x.name == h.name)
    {
        hosts.push(h.clone());
    }
    let mut found = Vec::new();
    for h in &hosts {
        let Some(ep) = endpoint(h) else { continue };
        let client = DockerClient::new(ep);
        match discover_from(&client, &DiscoverOptions::for_server(h.name.clone())).await {
            Ok(d) => {
                for s in d.services {
                    let card = s.to_core_service();
                    let key = format!(
                        "{}/{}",
                        card.server.as_deref().unwrap_or(&h.name),
                        s.container_name
                    );
                    let mut card = card;
                    card.server.get_or_insert_with(|| h.name.clone());
                    card.container
                        .get_or_insert_with(|| s.container_name.clone());
                    found.push((s.group, card, key));
                }
            }
            Err(e) => tracing::debug!(server = %h.name, error = %e, "docker discovery failed"),
        }
    }
    if found.is_empty() {
        return Ok(0);
    }
    let added = merge(&mut dash, found);
    let before = state.db.load_dashboard().await?;
    if added == 0 && before.settings.extra.get(SEEN_KEY) == dash.settings.extra.get(SEEN_KEY) {
        return Ok(0);
    }
    crate::secrets::seal(&mut dash, state.auth.secrets());
    crate::icons::store_inline_icons(&mut dash, &state.icon_dir())?;
    let summary = format!("docker discovery added {added} card(s)");
    match state
        .db
        .save_dashboard(
            &dash,
            Some(version),
            Some("docker discovery"),
            Some(&summary),
        )
        .await
    {
        Ok((v, _)) => {
            if added > 0 {
                tracing::info!(added, "docker discovery added cards");
                state.status.publish(LiveEvent::Config { version: v });
            }
            Ok(added)
        }
        // An edit landed meanwhile; try again next round.
        Err(homedit_db::DbError::Conflict { .. }) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// An endpoint as people write it: `unix:///var/run/docker.sock`, `tcp://host:2375`.
pub fn describe(ep: &DockerEndpoint) -> String {
    match ep {
        DockerEndpoint::Unix(p) => format!("unix://{}", p.display()),
        DockerEndpoint::Tcp { host, port } => format!("tcp://{host}:{port}"),
    }
}

/// The Docker address setting (`HOMEDIT_DOCKER_HOST`, or saved in Settings) as the host `local`; `None`
/// when empty or not an address.
pub fn configured_host(state: &AppState) -> Option<DockerHost> {
    let v = state.kit.settings.str("docker_host");
    if v.trim().is_empty() {
        return None;
    }
    let h = env_host(&v);
    if h.is_none() {
        tracing::warn!(value = %v, "the Docker address is not unix://path or tcp://host:port");
    }
    h
}

/// Starts the discovery loop.
pub fn spawn(state: AppState) -> tokio::task::JoinHandle<()> {
    let env = configured_host(&state);
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_once(&state, env.as_ref()).await {
                tracing::warn!(error = %e, "docker discovery round failed");
            }
            tokio::time::sleep(INTERVAL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use homedit_core::Service;

    #[test]
    fn env_hosts() {
        let h = env_host("unix:///var/run/docker.sock").unwrap();
        assert_eq!(h.socket.as_deref(), Some("/var/run/docker.sock"));
        let h = env_host("tcp://dockerproxy:2375").unwrap();
        assert_eq!(
            (h.host.as_deref(), h.port),
            (Some("dockerproxy"), Some(2375))
        );
        assert!(env_host("http://x").is_none());
    }

    #[test]
    fn merge_adds_once() {
        let mut d = Dashboard::default();
        let card = |n: &str| Service::new(n, None);
        let n = merge(
            &mut d,
            vec![
                ("Media".into(), card("Plex"), "local/plex".into()),
                ("Media".into(), card("Sonarr"), "local/sonarr".into()),
            ],
        );
        assert_eq!(n, 2);
        assert_eq!(d.groups[0].services.len(), 2);
        // Admin deletes Plex; it must not come back.
        d.groups[0].services.remove(0);
        let n = merge(
            &mut d,
            vec![("Media".into(), card("Plex"), "local/plex".into())],
        );
        assert_eq!(n, 0);
        assert_eq!(d.groups[0].services.len(), 1);
    }
}
