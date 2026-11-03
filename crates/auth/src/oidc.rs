//! OpenID Connect single sign-on (authorization code flow with PKCE).
//!
//! 1. `GET /api/auth/oidc/login` calls [`OidcProvider::begin`], which stores
//!    a pending login (state, nonce, PKCE verifier, return path) for ten
//!    minutes and redirects to the provider. The state is also put in a
//!    short-lived cookie, so a callback only completes in the browser that
//!    started it.
//! 2. `GET /api/auth/oidc/callback` calls [`OidcProvider::finish`], which
//!    exchanges the code, verifies the ID token (signature, issuer,
//!    audience, expiry, nonce) and returns the identity and groups.
//!
//! Provider discovery happens on first use and is cached, so homedit still
//! starts when the identity provider is down.
//!
//! Groups are read from the configured claim of the verified ID token, and
//! from the userinfo endpoint when the ID token does not carry them (as with
//! Authelia's default claims policy).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, HttpRequest, HttpResponse, IssuerUrl, Nonce, OAuth2TokenResponse,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};
use serde_json::Value;

use crate::config::OidcConfig;
use crate::error::AuthError;
use crate::role::Role;

type DiscoveredClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

struct Discovered {
    client: DiscoveredClient,
    userinfo_url: Option<String>,
}

struct Pending {
    nonce: Nonce,
    pkce: PkceCodeVerifier,
    return_to: String,
    created: Instant,
}

/// How long a started login may take.
const PENDING_TTL: Duration = Duration::from_secs(600);
/// Cap on concurrently pending logins.
const MAX_PENDING: usize = 1000;

/// The identity returned by a completed OIDC login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcIdentity {
    /// Stable id: `issuer|subject`.
    pub external_id: String,
    /// `preferred_username`, else `email`, else the subject.
    pub username: String,
    /// Group names from the configured claim.
    pub groups: Vec<String>,
}

/// Error from the HTTP adapter.
#[derive(Debug, thiserror::Error)]
enum HttpError {
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Http(#[from] axum::http::Error),
}

/// Bridges openidconnect's HTTP abstraction to the workspace `reqwest`.
async fn http_call(client: reqwest::Client, req: HttpRequest) -> Result<HttpResponse, HttpError> {
    let (parts, body) = req.into_parts();
    let resp = client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await?;
    let mut builder = axum::http::Response::builder().status(resp.status());
    for (k, v) in resp.headers() {
        builder = builder.header(k, v);
    }
    let body = resp.bytes().await?.to_vec();
    Ok(builder.body(body)?)
}

/// A configured OIDC provider.
pub struct OidcProvider {
    cfg: OidcConfig,
    http: reqwest::Client,
    discovered: tokio::sync::Mutex<Option<Arc<Discovered>>>,
    pending: Mutex<HashMap<String, Pending>>,
}

impl std::fmt::Debug for OidcProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OidcProvider")
            .field("issuer", &self.cfg.issuer_url)
            .finish_non_exhaustive()
    }
}

impl OidcProvider {
    /// Validates the config. Does not contact the provider.
    pub fn new(cfg: OidcConfig) -> Result<Self, AuthError> {
        IssuerUrl::new(cfg.issuer_url.clone())
            .map_err(|e| AuthError::BadRequest(format!("OIDC issuer: {e}")))?;
        RedirectUrl::new(cfg.redirect_url.clone())
            .map_err(|e| AuthError::BadRequest(format!("OIDC redirect URL: {e}")))?;
        let http = reqwest::Client::builder()
            // Following redirects from the token endpoint would be an SSRF vector.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| AuthError::Internal(e.to_string()))?;
        Ok(Self {
            cfg,
            http,
            discovered: tokio::sync::Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// The config this provider was built from.
    pub fn config(&self) -> &OidcConfig {
        &self.cfg
    }

    async fn discovered(&self) -> Result<Arc<Discovered>, AuthError> {
        let mut slot = self.discovered.lock().await;
        if let Some(d) = slot.as_ref() {
            return Ok(d.clone());
        }
        let http = |req| http_call(self.http.clone(), req);
        let issuer = IssuerUrl::new(self.cfg.issuer_url.clone())
            .map_err(|e| AuthError::Oidc(e.to_string()))?;
        let meta = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| AuthError::Oidc(format!("discovery: {e}")))?;
        let userinfo_url = meta.userinfo_endpoint().map(|u| u.url().to_string());
        let client = CoreClient::from_provider_metadata(
            meta,
            ClientId::new(self.cfg.client_id.clone()),
            self.cfg
                .client_secret
                .clone()
                .filter(|s| !s.is_empty())
                .map(ClientSecret::new),
        )
        .set_redirect_uri(
            RedirectUrl::new(self.cfg.redirect_url.clone())
                .map_err(|e| AuthError::Oidc(e.to_string()))?,
        );
        let d = Arc::new(Discovered {
            client,
            userinfo_url,
        });
        *slot = Some(d.clone());
        Ok(d)
    }

    /// Starts a login. Returns the provider URL to redirect to and the state
    /// value to pin in a cookie.
    pub async fn begin(&self, return_to: String) -> Result<(url::Url, String), AuthError> {
        let d = self.discovered().await?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut req = d
            .client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                CsrfToken::new_random,
                Nonce::new_random,
            )
            .set_pkce_challenge(challenge);
        for s in &self.cfg.scopes {
            req = req.add_scope(Scope::new(s.clone()));
        }
        let (url, state, nonce) = req.url();
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, p| p.created.elapsed() < PENDING_TTL);
        if pending.len() >= MAX_PENDING {
            return Err(AuthError::RateLimited(60));
        }
        pending.insert(
            state.secret().clone(),
            Pending {
                nonce,
                pkce: verifier,
                return_to,
                created: Instant::now(),
            },
        );
        Ok((url, state.secret().clone()))
    }

    /// Completes a login. Returns the identity and the path to send the
    /// browser back to.
    pub async fn finish(
        &self,
        state: &str,
        code: String,
    ) -> Result<(OidcIdentity, String), AuthError> {
        let pending = {
            let mut map = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            map.remove(state)
        };
        let pending = pending
            .filter(|p| p.created.elapsed() < PENDING_TTL)
            .ok_or_else(|| {
                AuthError::BadRequest("login expired or unknown, please try again".into())
            })?;
        let d = self.discovered().await?;
        let http = |req| http_call(self.http.clone(), req);
        let token = d
            .client
            .exchange_code(AuthorizationCode::new(code))
            .map_err(|e| AuthError::Oidc(e.to_string()))?
            .set_pkce_verifier(pending.pkce)
            .request_async(&http)
            .await
            .map_err(|e| AuthError::Oidc(format!("token exchange: {e}")))?;
        let id_token = token
            .id_token()
            .ok_or_else(|| AuthError::Oidc("provider returned no ID token".into()))?;
        let claims = id_token
            .claims(&d.client.id_token_verifier(), &pending.nonce)
            .map_err(|e| AuthError::Oidc(format!("ID token: {e}")))?;

        let subject = claims.subject().as_str().to_owned();
        let username = claims
            .preferred_username()
            .map(|u| u.as_str().to_owned())
            .or_else(|| claims.email().map(|e| e.as_str().to_owned()))
            .unwrap_or_else(|| subject.clone());
        let external_id = format!("{}|{}", claims.issuer().as_str(), subject);

        // The token's signature was verified above, so its payload can be
        // read directly for the non-standard groups claim.
        let mut groups = jwt_payload(&id_token.to_string())
            .and_then(|p| extract_groups(&p, &self.cfg.groups_claim));
        if groups.is_none()
            && let Some(url) = &d.userinfo_url
        {
            groups = self
                .userinfo_groups(url, token.access_token().secret(), &subject)
                .await;
        }
        Ok((
            OidcIdentity {
                external_id,
                username,
                groups: groups.unwrap_or_default(),
            },
            pending.return_to,
        ))
    }

    async fn userinfo_groups(
        &self,
        url: &str,
        access_token: &str,
        subject: &str,
    ) -> Option<Vec<String>> {
        let resp = self
            .http
            .get(url)
            .bearer_auth(access_token)
            .send()
            .await
            .ok()?;
        let body: Value = resp.error_for_status().ok()?.json().await.ok()?;
        // Userinfo must describe the same subject (OIDC Core 5.3.2).
        if body.get("sub").and_then(Value::as_str) != Some(subject) {
            tracing::warn!("userinfo subject mismatch; ignoring its groups");
            return None;
        }
        extract_groups(&body, &self.cfg.groups_claim)
    }

    /// Maps groups to a role, or refuses the login when `allowed_group` is
    /// set and the user is not in it.
    pub fn role_for(&self, groups: &[String]) -> Result<Role, AuthError> {
        role_for(&self.cfg, groups)
    }
}

/// See [`OidcProvider::role_for`].
pub fn role_for(cfg: &OidcConfig, groups: &[String]) -> Result<Role, AuthError> {
    if let Some(allowed) = &cfg.allowed_group
        && !groups.iter().any(|g| g == allowed)
        && cfg.admin_group.as_ref().is_none_or(|a| !groups.contains(a))
    {
        return Err(AuthError::Forbidden);
    }
    Ok(match &cfg.admin_group {
        Some(admin) if groups.iter().any(|g| g == admin) => Role::Admin,
        _ => cfg.default_role,
    })
}

fn jwt_payload(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?).ok()
}

/// Reads a groups claim that is a list of strings, a single string, or a
/// dotted path into nested objects (e.g. `realm_access.roles` for Keycloak).
pub fn extract_groups(claims: &Value, claim: &str) -> Option<Vec<String>> {
    let mut v = claims;
    for part in claim.split('.') {
        v = v.get(part)?;
    }
    match v {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        Value::String(s) => Some(
            s.split(',')
                .map(|g| g.trim().to_owned())
                .filter(|g| !g.is_empty())
                .collect(),
        ),
        _ => None,
    }
}

/// Only same-site relative paths are accepted as post-login destinations,
/// so the login flow cannot be used as an open redirect.
pub fn sanitize_return_to(path: Option<&str>) -> String {
    match path {
        Some(p)
            if p.starts_with('/')
                && !p.starts_with("//")
                && !p.starts_with("/\\")
                && !p.contains(['\r', '\n']) =>
        {
            p.to_owned()
        }
        _ => "/".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg() -> OidcConfig {
        serde_json::from_value(json!({
            "issuer_url": "https://id.example.com",
            "client_id": "homedit",
            "redirect_url": "https://dash.example.com/api/auth/oidc/callback",
            "admin_group": "admins"
        }))
        .unwrap()
    }

    #[test]
    fn groups_shapes() {
        assert_eq!(
            extract_groups(&json!({"groups": ["a", "b"]}), "groups").unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            extract_groups(&json!({"groups": "a, b"}), "groups").unwrap(),
            vec!["a", "b"]
        );
        assert_eq!(
            extract_groups(
                &json!({"realm_access": {"roles": ["admins"]}}),
                "realm_access.roles"
            )
            .unwrap(),
            vec!["admins"]
        );
        assert!(extract_groups(&json!({}), "groups").is_none());
    }

    #[test]
    fn roles_from_groups() {
        let c = cfg();
        assert_eq!(role_for(&c, &["admins".into()]).unwrap(), Role::Admin);
        assert_eq!(role_for(&c, &["users".into()]).unwrap(), Role::Viewer);
        let mut c = cfg();
        c.allowed_group = Some("homelab".into());
        assert!(role_for(&c, &["users".into()]).is_err());
        assert_eq!(role_for(&c, &["homelab".into()]).unwrap(), Role::Viewer);
        assert_eq!(role_for(&c, &["admins".into()]).unwrap(), Role::Admin);
    }

    #[test]
    fn return_to_is_relative_only() {
        assert_eq!(sanitize_return_to(Some("/settings?x=1")), "/settings?x=1");
        assert_eq!(sanitize_return_to(Some("//evil.example")), "/");
        assert_eq!(sanitize_return_to(Some("/\\evil.example")), "/");
        assert_eq!(sanitize_return_to(Some("https://evil.example")), "/");
        assert_eq!(sanitize_return_to(None), "/");
    }

    #[test]
    fn jwt_payload_decodes() {
        let p = URL_SAFE_NO_PAD.encode(br#"{"groups":["x"]}"#);
        assert_eq!(jwt_payload(&format!("h.{p}.s")).unwrap()["groups"][0], "x");
    }

    #[test]
    fn config_is_validated() {
        let mut c = cfg();
        c.issuer_url = "not a url".into();
        assert!(OidcProvider::new(c).is_err());
        assert!(OidcProvider::new(cfg()).is_ok());
    }
}
