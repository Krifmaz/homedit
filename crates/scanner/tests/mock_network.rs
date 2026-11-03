//! End-to-end scans against fake app servers on loopback addresses.

mod common;

use std::net::IpAddr;
use std::time::{Duration, Instant};

use common::{Route, localhost};
use scanner::{
    DockerEndpoint, PortList, PortSet, ScanError, ScanEvent, ScanPolicy, ScanReport, ScanRequest,
    Scanner, Scheme, Source, diff,
};

fn fast_policy() -> ScanPolicy {
    ScanPolicy {
        connects_per_second: 0,
        connect_timeout_ms: 500,
        http_timeout_ms: 2_000,
        ..ScanPolicy::default()
    }
}

fn ports(list: &[u16]) -> PortSet {
    let text: Vec<String> = list.iter().map(u16::to_string).collect();
    let ports: PortList = text.join(",").parse().unwrap();
    PortSet::List { ports }
}

fn request(target: &str, list: &[u16]) -> ScanRequest {
    let mut req = ScanRequest::new(vec![target.parse().unwrap()]);
    req.ports = ports(list);
    req.requested_by = Some("test-admin".into());
    req
}

fn service(report: &ScanReport, port: u16) -> &scanner::DiscoveredService {
    report
        .services()
        .map(|(_, s)| s)
        .find(|s| s.port == port)
        .unwrap_or_else(|| panic!("port {port} not in report: {report:#?}"))
}

#[tokio::test]
async fn identifies_apps_on_a_mock_network() {
    let ip = localhost();
    let jellyfin = common::jellyfin(ip).await;
    let plex = common::plex(ip).await;
    let sonarr = common::sonarr(ip).await;
    let qbit = common::qbittorrent(ip).await;
    let unknown = common::unknown_app(ip).await;
    let ssh = common::banner(ip, "SSH-2.0-OpenSSH_9.6\r\n").await;
    let closed = common::closed_port(ip).await;

    let list = [
        jellyfin.port(),
        plex.port(),
        sonarr.port(),
        qbit.port(),
        unknown.port(),
        ssh.port(),
        closed,
    ];
    let scanner = Scanner::new(fast_policy()).unwrap();
    let report = scanner
        .scan(request("127.0.0.1", &list), None)
        .await
        .unwrap();

    assert_eq!(report.requested_by.as_deref(), Some("test-admin"));
    assert_eq!(report.hosts.len(), 1);
    assert_eq!(
        report.hosts[0].services.len(),
        6,
        "closed port must not be listed"
    );
    assert!(report.services().all(|(_, s)| s.port != closed));

    let jf = service(&report, jellyfin.port());
    let app = jf.app.as_ref().expect("jellyfin identified");
    assert_eq!(app.id, "jellyfin");
    assert_eq!(app.widget.as_deref(), Some("jellyfin"));
    assert_eq!(app.version.as_deref(), Some("10.10.3"));
    assert_eq!(jf.http.as_ref().unwrap().scheme, Scheme::Http);
    assert_eq!(jf.source, Source::Scan);

    let px = service(&report, plex.port())
        .app
        .as_ref()
        .expect("plex identified");
    assert_eq!(px.id, "plex");
    assert_eq!(px.version.as_deref(), Some("1.41.3.9314-a0bfb8370"));

    let so = service(&report, sonarr.port())
        .app
        .as_ref()
        .expect("sonarr identified");
    assert_eq!(so.id, "sonarr");
    assert_eq!(so.version, None);

    let qb = service(&report, qbit.port())
        .app
        .as_ref()
        .expect("qbittorrent identified");
    assert_eq!(qb.id, "qbittorrent");
    assert_eq!(qb.version.as_deref(), Some("5.0.2"));

    let un = service(&report, unknown.port());
    assert!(un.app.is_none());
    let http = un.http.as_ref().unwrap();
    assert_eq!(http.title.as_deref(), Some("Family Photos"));
    assert_eq!(http.server.as_deref(), Some("nginx/1.27.0"));
    let icon = http.favicon.as_ref().expect("favicon");
    assert_eq!(icon.mime, "image/png");
    assert_eq!(icon.hash, scanner::fingerprint::favicon_hash(common::PNG));
    let suggestion = un.suggestion("127.0.0.1");
    assert_eq!(suggestion.name, "Family Photos");
    assert!(
        suggestion
            .favicon
            .unwrap()
            .starts_with("data:image/png;base64,")
    );

    let raw = service(&report, ssh.port());
    assert!(raw.http.is_none(), "SSH banner is not HTTP");
    assert!(raw.app.is_none());

    // "Add to dashboard" for a known app fills in the widget.
    let s = jf.suggestion("127.0.0.1");
    assert_eq!(s.name, "Jellyfin");
    assert_eq!(s.icon.as_deref(), Some("jellyfin.png"));
    assert_eq!(s.href, format!("http://127.0.0.1:{}/", jellyfin.port()));
    assert_eq!(s.widget.unwrap()["type"], "jellyfin");
}

#[tokio::test]
async fn detects_https_services() {
    let ip = localhost();
    let pve = common::https(
        ip,
        vec![
            Route::html("/", "pve1 - Proxmox Virtual Environment")
                .header("server", "pve-api-daemon/3.0"),
        ],
    )
    .await;
    let scanner = Scanner::new(fast_policy()).unwrap();
    let report = scanner
        .scan(request("127.0.0.1", &[pve.port()]), None)
        .await
        .unwrap();
    let svc = service(&report, pve.port());
    assert_eq!(svc.http.as_ref().unwrap().scheme, Scheme::Https);
    assert_eq!(svc.app.as_ref().unwrap().id, "proxmox");
    assert!(svc.suggestion("127.0.0.1").href.starts_with("https://"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn scans_a_cidr_range() {
    // Linux routes all of 127.0.0.0/8 to loopback, so these act as hosts.
    let a: IpAddr = "127.0.0.2".parse().unwrap();
    let b: IpAddr = "127.0.0.3".parse().unwrap();
    let on_a = common::sonarr(a).await;
    let port = on_a.port();
    // Same port on the second host, if it is free there.
    let on_b = common::http(
        b,
        vec![
            Route::html("/", "Radarr"),
            Route::new("/api/v3/system/status", 401, ""),
        ],
    );
    let on_b = on_b.await;

    let scanner = Scanner::new(fast_policy()).unwrap();
    let report = scanner
        .scan(request("127.0.0.0/29", &[port, on_b.port()]), None)
        .await
        .unwrap();
    assert_eq!(report.scanned_addresses.len(), 6);
    let found: Vec<(String, String)> = report
        .services()
        .filter_map(|(h, s)| s.app.as_ref().map(|a| (h.ip.to_string(), a.id.clone())))
        .collect();
    assert!(
        found.contains(&("127.0.0.2".into(), "sonarr".into())),
        "{found:?}"
    );
    assert!(
        found.contains(&("127.0.0.3".into(), "radarr".into())),
        "{found:?}"
    );
}

#[tokio::test]
async fn refuses_public_targets_before_scanning() {
    let scanner = Scanner::new(fast_policy()).unwrap();
    let mut req = ScanRequest::new(vec![
        "10.0.0.1".parse().unwrap(),
        "8.8.8.8".parse().unwrap(),
    ]);
    req.ports = ports(&[80]);
    let err = scanner.scan(req, None).await.unwrap_err();
    assert!(matches!(err, ScanError::AddressNotAllowed { .. }), "{err}");
    assert!(
        scanner
            .fingerprint_service("1.1.1.1".parse().unwrap(), 80)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn refuses_oversized_scans() {
    let policy = ScanPolicy {
        max_probes: 1_000,
        ..fast_policy()
    };
    let scanner = Scanner::new(policy).unwrap();
    let mut req = ScanRequest::new(vec!["127.0.0.1".parse().unwrap()]);
    req.ports = PortSet::Full;
    assert!(matches!(
        scanner.scan(req, None).await,
        Err(ScanError::TooLarge(_))
    ));
}

#[tokio::test]
async fn reports_progress_events() {
    let ip = localhost();
    let jellyfin = common::jellyfin(ip).await;
    let closed = common::closed_port(ip).await;
    let scanner = Scanner::new(fast_policy()).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let report = scanner
        .scan(request("127.0.0.1", &[jellyfin.port(), closed]), Some(tx))
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    assert!(matches!(
        events.first(),
        Some(ScanEvent::Started {
            addresses: 1,
            ports: 2,
            probes: 2
        })
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ScanEvent::PortOpen { port, .. } if *port == jellyfin.port()))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, ScanEvent::Service { service, .. } if service.app.is_some()))
    );
    assert!(matches!(
        events.last(),
        Some(ScanEvent::Finished {
            open: 1,
            identified: 1,
            ..
        })
    ));
    // Events serialize for a server-sent-events stream.
    assert!(
        serde_json::to_string(&events[0])
            .unwrap()
            .contains(r#""event":"started""#)
    );
    assert_eq!(report.probes_sent, 2);
}

#[tokio::test]
async fn rate_limit_slows_the_scan() {
    let ip = localhost();
    let policy = ScanPolicy {
        connects_per_second: 20,
        ..fast_policy()
    };
    let scanner = Scanner::new(policy).unwrap();
    let mut list = Vec::new();
    for _ in 0..22 {
        list.push(common::closed_port(ip).await);
    }
    let mut req = request("127.0.0.1", &list);
    req.fingerprint = false;
    let start = Instant::now();
    scanner.scan(req, None).await.unwrap();
    // A burst of 2, then 20 per second: about one second for 22 probes.
    assert!(
        start.elapsed() >= Duration::from_millis(900),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn rescan_flags_new_and_gone_services() {
    let ip = localhost();
    let jellyfin = common::jellyfin(ip).await;
    let sonarr = common::sonarr(ip).await;
    let qbit_port = common::closed_port(ip).await;
    let list = [jellyfin.port(), sonarr.port(), qbit_port];
    let scanner = Scanner::new(fast_policy()).unwrap();

    let first = scanner
        .scan(request("127.0.0.1", &list), None)
        .await
        .unwrap();
    drop(sonarr);
    let qbit = common::http(
        ip,
        vec![
            Route::html("/", "qBittorrent WebUI"),
            Route::new("/api/v2/app/version", 200, "v5.0.2"),
        ],
    )
    .await;
    // The new server got a fresh port; rescan with it included.
    let mut list2 = list.to_vec();
    list2.push(qbit.port());
    let second = scanner
        .scan(request("127.0.0.1", &list2), None)
        .await
        .unwrap();

    let changes = diff(&first, &second, scanner.catalogue());
    let msgs = changes.messages();
    assert!(
        msgs.contains(&format!("qBittorrent found on 127.0.0.1:{}", qbit.port())),
        "{msgs:?}"
    );
    assert!(
        msgs.iter()
            .any(|m| m.starts_with("Sonarr on 127.0.0.1:") && m.ends_with("stopped answering")),
        "{msgs:?}"
    );
    assert!(!msgs.iter().any(|m| m.contains("Jellyfin")), "{msgs:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn reads_published_ports_from_docker() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let ip = localhost();
    let jellyfin = common::jellyfin(ip).await;
    let jf_port = jellyfin.port();
    let containers = format!(
        r#"[
  {{"Id":"4f2a9c1d8e7b6a5f4e3d2c1b","Names":["/jellyfin"],"Image":"jellyfin/jellyfin:10.10.3","State":"running",
    "Labels":{{"homepage.group":"Media","com.docker.compose.project":"media"}},
    "Ports":[{{"IP":"0.0.0.0","PrivatePort":8096,"PublicPort":{jf_port},"Type":"tcp"}},
             {{"IP":"::","PrivatePort":8096,"PublicPort":{jf_port},"Type":"tcp"}},
             {{"PrivatePort":8920,"Type":"tcp"}},
             {{"IP":"0.0.0.0","PrivatePort":1900,"PublicPort":1900,"Type":"udp"}}]}},
  {{"Id":"aaaaaaaaaaaaaaaaaaaaaaaa","Names":["/sonarr"],"Image":"lscr.io/linuxserver/sonarr:latest","State":"running",
    "Labels":null,
    "Ports":[{{"IP":"10.9.9.9","PrivatePort":8989,"PublicPort":8989,"Type":"tcp"}}]}},
  {{"Id":"bbbbbbbbbbbbbbbbbbbbbbbb","Names":["/redis"],"Image":"redis:7","State":"running","Labels":{{}},"Ports":[]}}
]"#
    );

    let dir = std::env::temp_dir().join(format!("homedit-scanner-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("docker.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap();
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        // Answer in two chunks to exercise chunked decoding.
        let (a, b) = containers.split_at(containers.len() / 2);
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n",
            a.len(),
            b.len()
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
        req
    });

    let scanner = Scanner::new(fast_policy()).unwrap();
    let endpoint: DockerEndpoint = format!("unix://{}", socket.display()).parse().unwrap();
    let host = scanner
        .docker_services(&endpoint, ip, Some("docker-host".into()), true)
        .await
        .unwrap();
    let request_line = server.await.unwrap();
    assert!(
        request_line.starts_with("GET /containers/json HTTP/1.1"),
        "{request_line}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // Only the Jellyfin port: Sonarr is bound to another address, the UDP
    // and unpublished ports are skipped, the IPv6 duplicate is merged.
    assert_eq!(host.services.len(), 1, "{host:#?}");
    let svc = &host.services[0];
    assert_eq!(svc.port, jf_port);
    assert_eq!(svc.source, Source::Docker);
    let c = svc.container.as_ref().unwrap();
    assert_eq!(c.name, "jellyfin");
    assert_eq!(c.id, "4f2a9c1d8e7b");
    let app = svc.app.as_ref().unwrap();
    assert_eq!(app.id, "jellyfin");
    assert_eq!(
        app.version.as_deref(),
        Some("10.10.3"),
        "confirmed over HTTP"
    );
    assert_eq!(
        svc.suggestion(&host.host_label()).href,
        format!("http://docker-host:{jf_port}/")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn docker_ports_without_fingerprinting() {
    let catalogue = scanner::Catalogue::builtin();
    let containers = r#"[{"Id":"cccccccccccccccc","Names":["/plex"],"Image":"plexinc/pms-docker","State":"running","Labels":{"homepage.name":"Plex"},"Ports":[{"IP":"0.0.0.0","PrivatePort":32400,"PublicPort":32400,"Type":"tcp"}]}]"#;
    let dir = std::env::temp_dir().join(format!("homedit-scanner-test2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("docker.sock");
    let _ = std::fs::remove_file(&socket);
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{containers}",
            containers.len()
        );
        stream.write_all(resp.as_bytes()).await.unwrap();
    });
    let endpoint = scanner::DockerEndpoint::Unix(socket);
    let ports = scanner::docker::published_ports(&endpoint, catalogue)
        .await
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].app.as_ref().unwrap().id, "plex");
    assert_eq!(
        ports[0].labels.get("homepage.name").map(String::as_str),
        Some("Plex")
    );
    assert_eq!(ports[0].host_ip, None);
}
