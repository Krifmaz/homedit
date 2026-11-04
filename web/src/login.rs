//! The sign-in page, and the first-run page that creates the first admin. Shown instead of the dashboard when
//! the server needs a sign-in, when no accounts exist yet, or when a visitor asked for it.

use crate::client;
use crate::model::{AuthMethods, js_encode};
use crate::state::app;
use kit_leptos::api::ApiError;
use kit_leptos::rt;
use leptos::prelude::*;
use serde_json::Value;

/// The shortest new password the server takes.
pub const MIN_PASSWORD: usize = 10;

/// The sentence for `?authError=<code>` after a failed single sign-on.
pub fn auth_error(code: &str) -> Option<String> {
    if code.is_empty() {
        return None;
    }
    Some(match code {
        "access_denied" => "Sign-in was cancelled or denied.".into(),
        "disabled" => "This account is disabled.".into(),
        "no_account" => "No account is linked to that identity.".into(),
        c => format!("Single sign-on failed ({c})."),
    })
}

/// The sentence for a failed sign-in or setup.
pub fn login_error(e: &ApiError) -> String {
    match e.data.get("error").and_then(Value::as_str) {
        Some("invalid_credentials") => "Wrong username or password.".into(),
        Some("rate_limited") => match e.data.get("retryAfter").and_then(Value::as_f64) {
            Some(s) if s > 0.0 => {
                format!("Too many attempts. Try again in {} min.", (s / 60.0).ceil())
            }
            _ => "Too many attempts. Try again later.".into(),
        },
        Some("disabled") => "Password sign-in is turned off on this server.".into(),
        _ if !e.message.is_empty() => e.message.clone(),
        _ => "Sign-in failed.".into(),
    }
}

fn query_param(key: &str) -> String {
    let q = rt::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    q.trim_start_matches('?')
        .split('&')
        .filter_map(|p| p.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}

/// The sign-in or setup page.
#[component]
pub fn Login() -> impl IntoView {
    let a = app();
    let me = a.me.get_untracked();
    let setup = me.as_ref().is_some_and(|m| m.setup_required);
    let methods = me
        .as_ref()
        .map(|m| m.methods.clone())
        .unwrap_or(AuthMethods {
            local: true,
            ..AuthMethods::default()
        });
    let user = RwSignal::new(if setup {
        "admin".to_string()
    } else {
        String::new()
    });
    let pass = RwSignal::new(String::new());
    let confirm = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(auth_error(&query_param("authError")));
    let too_short = move || {
        setup && !pass.with(String::is_empty) && pass.with(|p| p.chars().count() < MIN_PASSWORD)
    };
    let mismatch = move || setup && !confirm.with(String::is_empty) && confirm.get() != pass.get();
    let can = move || {
        !busy.get()
            && !user.with(|u| u.trim().is_empty())
            && !pass.with(String::is_empty)
            && (!setup
                || (pass.with(|p| p.chars().count() >= MIN_PASSWORD)
                    && confirm.get() == pass.get()))
    };
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        if !can() {
            return;
        }
        busy.set(true);
        error.set(None);
        rt::spawn(async move {
            let path = if setup { "/auth/setup" } else { "/auth/login" };
            match client::session(path, user.get_untracked().trim(), &pass.get_untracked()).await {
                Ok(_) => a.refresh_session(),
                Err(e) => {
                    error.set(Some(login_error(&e)));
                    busy.set(false);
                }
            }
        });
    };
    let return_to = {
        let w = rt::window();
        let path = w
            .as_ref()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_else(|| "/".into());
        js_encode(&path)
    };
    let oidc = methods.oidc.clone().filter(|_| !setup);
    let local = setup || methods.local;
    view! {
        <div class="auth-wrap">
            <form class="auth-card" on:submit=submit aria-labelledby="login-title">
                <div class="brand" id="login-title">{if setup { "Create the admin account" } else { "Sign in" }}</div>
                {setup.then(|| view! { <p class="muted small" style="text-align:center">"No accounts exist yet. The account you create here is the first admin."</p> })}
                <div class="error-box" role="alert" hidden=move || error.with(Option::is_none)>{move || error.get()}</div>
                {local.then(|| view! {
                    <label class="field"><span>"Username"</span>
                        <input class="input" autocomplete="username" autofocus prop:value=move || user.get() on:input=move |e| user.set(event_target_value(&e))/></label>
                    <label class="field"><span>"Password"</span>
                        <input class="input" type="password" autocomplete=if setup { "new-password" } else { "current-password" }
                            prop:value=move || pass.get() on:input=move |e| pass.set(event_target_value(&e))/>
                        {setup.then(|| view! { <small class:err=too_short>{format!("At least {MIN_PASSWORD} characters.")}</small> })}
                    </label>
                    {setup.then(|| view! {
                        <label class="field"><span>"Confirm password"</span>
                            <input class="input" type="password" autocomplete="new-password" prop:value=move || confirm.get() on:input=move |e| confirm.set(event_target_value(&e))/>
                            {move || mismatch().then(|| view! { <small class="err">"Passwords don't match."</small> })}
                        </label>
                    })}
                    <button type="submit" class="btn primary" style="padding:12px" disabled=move || !can()>
                        {move || if busy.get() { "Please wait…" } else if setup { "Create admin and sign in" } else { "Sign in" }}
                    </button>
                })}
                {oidc.map(|label| view! {
                    {local.then(|| view! { <div class="muted small" style="text-align:center">"or"</div> })}
                    <a class="btn" style="padding:12px;justify-content:center" href=format!("/api/auth/oidc/login?returnTo={return_to}")>{label}</a>
                })}
                {(!setup && !methods.local && methods.oidc.is_none()).then(|| view! {
                    <p class="muted small">"Your reverse proxy signs you in. Reload the page after signing in there."</p>
                })}
                {move || a.login_optional().then(|| view! {
                    <button type="button" class="btn ghost" on:click=move |_| a.want_login.set(false)>"← Back to the dashboard"</button>
                })}
            </form>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_sentences() {
        assert_eq!(auth_error(""), None);
        assert_eq!(auth_error("disabled").unwrap(), "This account is disabled.");
        assert_eq!(auth_error("x").unwrap(), "Single sign-on failed (x).");
        let e = |d: Value| ApiError {
            status: 400,
            message: "m".into(),
            data: d,
        };
        assert_eq!(
            login_error(&e(json!({ "error": "invalid_credentials" }))),
            "Wrong username or password."
        );
        assert_eq!(
            login_error(&e(json!({ "error": "rate_limited", "retryAfter": 90 }))),
            "Too many attempts. Try again in 2 min."
        );
        assert_eq!(login_error(&e(json!({}))), "m");
    }
}
