//! Tests for Docker discovery against a mock Docker API on a Unix socket.

use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use homedit_discovery::{
    ContainerState, DiscoverOptions, DiscoveryError, DockerClient, DockerEndpoint, Health,
    discover, discover_from,
};
use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/containers.json");

/// Starts a mock Docker API on a Unix socket. Records every request as
/// "METHOD path?query" so tests can check that only GETs are sent.
async fn mock_docker() -> (tempfile::TempDir, PathBuf, Arc<Mutex<Vec<String>>>) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("docker.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let log = log.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                    let log = log.clone();
                    async move {
                        let target = req.uri().path_and_query().map(|p| p.to_string());
                        log.lock().unwrap().push(format!(
                            "{} {}",
                            req.method(),
                            target.clone().unwrap_or_default()
                        ));
                        let (status, body) = match req.uri().path() {
                            "/_ping" => (StatusCode::OK, Bytes::from_static(b"OK")),
                            "/containers/json" => {
                                (StatusCode::OK, Bytes::from_static(FIXTURE.as_bytes()))
                            }
                            _ => (
                                StatusCode::NOT_FOUND,
                                Bytes::from_static(br#"{"message":"page not found"}"#),
                            ),
                        };
                        Ok::<_, Infallible>(
                            Response::builder()
                                .status(status)
                                .body(Full::new(body))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (dir, socket, seen)
}

#[tokio::test]
async fn lists_containers_over_unix_socket_with_get_only() {
    let (_dir, socket, seen) = mock_docker().await;
    let client = DockerClient::new(DockerEndpoint::Unix(socket));
    client.ping().await.unwrap();
    let containers = client.list_containers(true).await.unwrap();
    assert_eq!(containers.len(), 5);
    assert_eq!(containers[0].name(), "sonarr");
    assert_eq!(containers[0].state(), ContainerState::Running);
    assert_eq!(containers[0].health(), Some(Health::Healthy));
    assert_eq!(containers[2].state(), ContainerState::Exited);

    let err = client.get_bytes("/nope").await.unwrap_err();
    assert!(matches!(err, DiscoveryError::Status(404, _)), "{err}");

    let requests = seen.lock().unwrap().clone();
    assert_eq!(
        requests,
        ["GET /_ping", "GET /containers/json?all=true", "GET /nope"]
    );
}

#[tokio::test]
async fn discovers_services_from_labels() {
    let (_dir, socket, _seen) = mock_docker().await;
    let client = DockerClient::new(DockerEndpoint::Unix(socket));
    let found = discover_from(&client, &DiscoverOptions::for_server("local"))
        .await
        .unwrap();

    // Exited Plex, the half-labelled container and postgres are skipped;
    // Radarr sorts before Sonarr by weight.
    let names: Vec<_> = found
        .services
        .iter()
        .map(|s| s.service.name.as_str())
        .collect();
    assert_eq!(names, ["Radarr", "Sonarr"]);

    let sonarr = &found.services[1];
    assert_eq!(sonarr.group, "Media");
    assert_eq!(sonarr.container_name, "sonarr");
    assert_eq!(sonarr.service.server.as_deref(), Some("local"));
    assert_eq!(sonarr.service.container.as_deref(), Some("sonarr"));
    assert_eq!(sonarr.service.weight, Some(2));
    assert_eq!(sonarr.service.widgets[0].kind, "sonarr");
    assert_eq!(
        sonarr.service.widgets[0].options["fields"],
        json!(["wanted", "queued"])
    );

    let radarr = &found.services[0];
    // An explicit `homepage.server` label wins over the default.
    assert_eq!(radarr.service.server.as_deref(), Some("nas"));
    // Instance labels are ignored without an instance name.
    assert_eq!(radarr.service.href, None);

    let core = sonarr.to_core_service();
    assert_eq!(
        core.widget.unwrap(),
        json!({"type": "sonarr", "url": "http://sonarr:8989", "key": "abc123", "fields": ["wanted", "queued"]})
    );

    assert_eq!(found.warnings.len(), 1);
    assert_eq!(found.warnings[0].path, "half-labelled");
}

#[tokio::test]
async fn instance_and_stopped_options() {
    let containers: Vec<homedit_discovery::Container> = serde_json::from_str(FIXTURE).unwrap();
    let options = DiscoverOptions {
        server: None,
        instance: Some("internal".into()),
        include_stopped: true,
    };
    let found = discover(&containers, &options);
    let names: Vec<_> = found
        .services
        .iter()
        .map(|s| s.service.name.as_str())
        .collect();
    assert_eq!(names, ["Plex", "Radarr", "Sonarr"]);
    assert_eq!(found.services[0].state, ContainerState::Exited);
    assert_eq!(
        found.services[1].service.href.as_deref(),
        Some("http://radarr:7878")
    );
    assert_eq!(found.services[2].service.server, None);
}

#[tokio::test]
async fn reports_missing_socket_and_timeouts() {
    let client = DockerClient::new(DockerEndpoint::Unix("/nonexistent/docker.sock".into()));
    assert!(matches!(
        client.ping().await,
        Err(DiscoveryError::Connect(..))
    ));

    // A server that accepts but never answers.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            held.push(stream);
        }
    });
    let client = DockerClient::new(DockerEndpoint::Tcp {
        host: "127.0.0.1".into(),
        port,
    })
    .with_timeout(std::time::Duration::from_millis(200));
    assert!(matches!(
        client.ping().await,
        Err(DiscoveryError::Timeout(_))
    ));
}

#[test]
fn endpoint_from_docker_yaml() {
    let socket = homedit_import::DockerServer {
        name: "local".into(),
        socket: Some("/var/run/docker.sock".into()),
        ..Default::default()
    };
    assert_eq!(
        DockerEndpoint::from_server(&socket).unwrap().to_string(),
        "unix:///var/run/docker.sock"
    );
    let tcp = homedit_import::DockerServer {
        name: "proxy".into(),
        host: Some("dockerproxy".into()),
        ..Default::default()
    };
    assert_eq!(
        DockerEndpoint::from_server(&tcp).unwrap(),
        DockerEndpoint::Tcp {
            host: "dockerproxy".into(),
            port: 2375
        }
    );
    assert_eq!(DockerEndpoint::from_server(&Default::default()), None);
}
