//! The guarded HTTP client against real local servers.

use axum::Router;
use axum::response::Redirect;
use axum::routing::get;
use homedit_auth::ssrf::{ClientOptions, GuardedClient, SsrfGuard};

async fn serve(app: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

#[tokio::test]
async fn allowed_host_and_redirect_checks() {
    let target = serve(Router::new().route("/", get(|| async { "secret internal page" }))).await;
    let redirector =
        serve(
            Router::new()
                .route("/ok", get(|| async { "fine" }))
                .route(
                    "/away",
                    get(move || async move {
                        Redirect::temporary(&format!("http://localhost:{target}/"))
                    }),
                )
                .route("/same", get(|| async { Redirect::temporary("/ok") })),
        )
        .await;

    let guard = SsrfGuard::from_urls([format!("127.0.0.1:{redirector}").as_str()]);
    let client = GuardedClient::new(guard.clone(), ClientOptions::default()).unwrap();

    let ok = client
        .get(&format!("http://127.0.0.1:{redirector}/ok"))
        .unwrap()
        .send()
        .await
        .unwrap();
    assert_eq!(ok.text().await.unwrap(), "fine");

    let same = client
        .get(&format!("http://127.0.0.1:{redirector}/same"))
        .unwrap()
        .send()
        .await
        .unwrap();
    assert_eq!(
        same.text().await.unwrap(),
        "fine",
        "same-host redirects are followed"
    );

    assert!(
        client.get(&format!("http://127.0.0.1:{target}/")).is_err(),
        "other port refused up front"
    );
    assert!(client.get(&format!("http://localhost:{target}/")).is_err());

    let err = client
        .get(&format!("http://127.0.0.1:{redirector}/away"))
        .unwrap()
        .send()
        .await
        .unwrap_err();
    assert!(
        err.is_redirect(),
        "redirect to an unlisted host is refused: {err}"
    );

    // Hostnames go through the guarded resolver.
    guard.replace(["localhost"]);
    let r = client
        .get(&format!("http://localhost:{target}/"))
        .unwrap()
        .send()
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "secret internal page");
}
