//! End-to-end tests for the gethomepage importer against a fixture folder.

use std::collections::HashMap;
use std::path::Path;

use homedit_import::{ImportError, ImportSources, MapResolver, NoResolver, import_sources};
use serde_json::json;

fn fixture_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/homepage"
    ))
}

fn resolver() -> MapResolver {
    MapResolver(HashMap::from([
        ("HOMEPAGE_VAR_PLEX_TOKEN".into(), "plex-token".into()),
        ("HOMEPAGE_FILE_JELLYFIN_KEY".into(), "/secrets/jf".into()),
        ("/secrets/jf".into(), "jf-key\n".into()),
    ]))
}

fn load() -> homedit_import::ImportedConfig {
    let sources = ImportSources::from_dir(fixture_dir()).expect("fixture dir readable");
    import_sources(&sources, &resolver()).expect("fixture imports")
}

#[test]
fn imports_service_groups_and_nested_groups() {
    let config = load();
    let names: Vec<_> = config.services.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["Media", "Downloads"]);

    let media = &config.services[0];
    let plex = &media.services[0];
    assert_eq!(plex.name, "Plex");
    assert_eq!(plex.href.as_deref(), Some("http://plex.lan:32400/web"));
    assert_eq!(plex.server.as_deref(), Some("local"));
    assert_eq!(plex.widgets.len(), 1);
    assert_eq!(plex.widgets[0].kind, "plex");
    assert_eq!(plex.widgets[0].url(), Some("http://plex.lan:32400"));
    assert_eq!(plex.widgets[0].options["key"], json!("plex-token"));
    assert_eq!(plex.widgets[0].secret_fields(), ["key"]);

    let jellyfin = &media.services[1];
    assert_eq!(
        jellyfin.site_monitor.as_deref(),
        Some("https://jellyfin.lan")
    );
    assert_eq!(jellyfin.weight, Some(10));
    assert_eq!(jellyfin.widgets.len(), 2);
    assert_eq!(jellyfin.widgets[0].options["key"], json!("jf-key"));
    assert_eq!(jellyfin.widgets[1].kind, "customapi");
    assert_eq!(
        jellyfin.widgets[1].options["mappings"],
        json!([{"field": "Version", "label": "Version"}])
    );

    let downloads = &config.services[1];
    assert_eq!(downloads.groups.len(), 1);
    let arr = &downloads.groups[0];
    assert_eq!(arr.name, "Arr");
    assert_eq!(arr.services.len(), 2);
    assert_eq!(arr.services[0].ping.as_deref(), Some("sonarr.lan"));
    assert_eq!(arr.services[1].show_stats, Some(true));

    let qbit = &downloads.services[0];
    assert_eq!(qbit.widgets[0].secret_fields(), ["password"]);
    // "Broken" is skipped with a warning, "Bare" is kept as an empty card.
    let svc_names: Vec<_> = downloads.services.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(svc_names, ["qBittorrent", "Bare"]);

    let all = config.all_services();
    assert_eq!(all.len(), 6);
    assert_eq!(all[2].0, ["Downloads"]);
    assert_eq!(all[4].0, ["Downloads", "Arr"]);
    assert_eq!(all[4].1.name, "Sonarr");
}

#[test]
fn reports_warnings_for_bad_entries_and_missing_vars() {
    let config = load();
    let messages: Vec<String> = config
        .warnings
        .iter()
        .map(|w| format!("{}:{}:{}", w.file, w.path, w.message))
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("Downloads/Broken")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.starts_with("services.yaml:[2]")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("HOMEPAGE_VAR_OWM")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("docker.yaml:broken")),
        "{messages:?}"
    );
}

#[test]
fn imports_bookmarks_in_both_shapes() {
    let config = load();
    assert_eq!(config.bookmarks.len(), 2);
    let dev = &config.bookmarks[0];
    assert_eq!(dev.bookmarks[0].abbr.as_deref(), Some("GH"));
    assert_eq!(dev.bookmarks[1].href.as_deref(), Some("https://docs.rs"));
    let reddit = &config.bookmarks[1].bookmarks[0];
    assert_eq!(reddit.description.as_deref(), Some("The front page"));
    assert_eq!(reddit.extra["target"], json!("_blank"));
}

#[test]
fn imports_widgets_settings_and_docker() {
    let config = load();
    let kinds: Vec<_> = config.widgets.iter().map(|w| w.kind.as_str()).collect();
    assert_eq!(kinds, ["resources", "search", "datetime", "greeting"]);
    assert_eq!(config.widgets[0].options["disk"], json!("/"));

    let s = &config.settings;
    assert_eq!(s.title.as_deref(), Some("My Lab"));
    assert_eq!(s.theme.as_deref(), Some("dark"));
    assert_eq!(s.header_style.as_deref(), Some("boxed"));
    assert_eq!(s.background.as_ref().unwrap()["blur"], json!("sm"));
    assert_eq!(s.layout.len(), 2);
    assert_eq!(s.layout[0].group, "Media");
    assert_eq!(s.layout[0].columns, Some(4));
    assert_eq!(s.layout[1].header, Some(false));
    assert_eq!(s.layout[1].initially_collapsed, Some(true));
    assert!(s.extra.contains_key("providers"));

    let names: Vec<_> = config.docker.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["broken", "local", "remote"]);
    assert_eq!(
        config.docker[1].socket.as_deref(),
        Some("/var/run/docker.sock")
    );
    assert_eq!(config.docker[2].port, Some(2375));
    assert!(config.docker[2].swarm);
}

#[test]
fn layout_as_list_and_empty_files() {
    let sources = ImportSources {
        settings: Some("layout:\n  - B:\n      columns: 2\n  - A:\n".into()),
        services: Some("# nothing yet\n".into()),
        ..ImportSources::default()
    };
    let config = import_sources(&sources, &NoResolver).unwrap();
    let groups: Vec<_> = config
        .settings
        .layout
        .iter()
        .map(|l| l.group.as_str())
        .collect();
    assert_eq!(groups, ["B", "A"]);
    assert!(config.services.is_empty());
}

#[test]
fn rejects_invalid_yaml_and_wrong_shapes() {
    let bad_yaml = ImportSources {
        services: Some("- Media:\n  - [unclosed".into()),
        ..ImportSources::default()
    };
    assert!(matches!(
        import_sources(&bad_yaml, &NoResolver),
        Err(ImportError::Yaml { .. })
    ));

    let wrong_shape = ImportSources {
        services: Some("Media: {}".into()),
        ..ImportSources::default()
    };
    assert!(matches!(
        import_sources(&wrong_shape, &NoResolver),
        Err(ImportError::Shape { .. })
    ));
}

#[test]
fn empty_dir_is_an_error_and_set_by_name_works() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        ImportSources::from_dir(dir.path()),
        Err(ImportError::NothingFound(_))
    ));
    let mut sources = ImportSources::default();
    assert!(sources.set_by_name("docker.yml", "a:\n  socket: /x\n".into()));
    assert!(!sources.set_by_name("custom.js", String::new()));
    assert!(sources.set_by_name("custom.css", "body{}".into()));
    let config = import_sources(&sources, &NoResolver).unwrap();
    assert_eq!(config.docker[0].socket.as_deref(), Some("/x"));
}

#[test]
fn converts_into_core_dashboard() {
    use homedit_core::model::Theme;
    let sources = ImportSources::from_dir(fixture_dir()).unwrap();
    let report = homedit_import::import_dashboard(&sources, &resolver()).unwrap();
    let dash = &report.dashboard;

    assert_eq!(dash.settings.title, "My Lab");
    assert_eq!(dash.settings.theme, Theme::Dark);
    assert_eq!(dash.settings.search_provider.as_deref(), Some("duckduckgo"));
    assert_eq!(dash.settings.extra["headerStyle"], json!("boxed"));
    let bg = dash.settings.background.as_ref().unwrap();
    assert_eq!(bg.image, "https://example.com/bg.jpg");
    assert_eq!(bg.opacity, Some(50));
    assert!(
        dash.settings
            .custom_css
            .as_deref()
            .unwrap()
            .contains(".card")
    );

    assert_eq!(dash.groups[0].layout.columns, Some(4));
    assert_eq!(dash.groups[1].layout.initially_collapsed, Some(true));
    let plex = &dash.groups[0].services[0];
    assert_eq!(
        plex.widget.as_ref().unwrap(),
        &json!({"type": "plex", "url": "http://plex.lan:32400", "key": "plex-token"})
    );
    let jellyfin = &dash.groups[0].services[1];
    assert_eq!(jellyfin.widget.as_ref().unwrap()["type"], json!("jellyfin"));
    assert_eq!(jellyfin.extra["widgets"][0]["type"], json!("customapi"));
    assert_eq!(jellyfin.extra["weight"], json!(10));
    assert_eq!(dash.groups[1].groups[0].services[0].name, "Sonarr");
    assert_eq!(dash.services().count(), 6);

    // `target` is not a core bookmark field: dropped with a warning.
    assert_eq!(
        dash.bookmark_groups[1].bookmarks[0].href,
        "https://reddit.com/"
    );
    assert!(report.warnings.iter().any(|w| w.message.contains("target")));

    assert_eq!(dash.info_widgets.len(), 4);
    assert_eq!(dash.docker_hosts[2].extra["swarm"], json!(true));

    // The dashboard survives a JSON round trip (what the database stores).
    let text = serde_json::to_string(dash).unwrap();
    let back: homedit_core::Dashboard = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, dash);
}
