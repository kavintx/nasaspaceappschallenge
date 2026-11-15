//! Accounts: /login, /signup, /account (profile), logout.
//!
//! The research backend owns the accounts (argon2-hashed, sessions table).
//! The browser only ever sees an HttpOnly session cookie; this server holds
//! the token and replays it to the backend — the same shape as every other
//! backend call, so no site token ever reaches the browser.

use crate::AppState;
use askama::Template;
use axum::extract::{Form, State};
use axum::http::header::{self, HeaderMap, HeaderValue};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

const COOKIE: &str = "burnscan_session";
/// 30 days — matches the backend's own session lifetime.
const MAX_AGE: i64 = 30 * 24 * 3600;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginPage {
    active: &'static str,
    page_title: String,
    context_line: String,
    error: String,
    backend_on: bool,
}

#[derive(Template)]
#[template(path = "signup.html")]
struct SignupPage {
    active: &'static str,
    page_title: String,
    context_line: String,
    error: String,
    backend_on: bool,
}

#[derive(Template)]
#[template(path = "account.html")]
struct AccountPage {
    active: &'static str,
    page_title: String,
    context_line: String,
    name: String,
    email: String,
    initial: char,
}

#[derive(Debug, Deserialize)]
pub struct AuthForm {
    email: String,
    password: String,
    #[serde(default)]
    name: Option<String>,
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| p.trim().strip_prefix(COOKIE)?.strip_prefix('=').map(str::to_string))
        .filter(|v| !v.is_empty())
}

fn cookie_header(token: &str) -> HeaderValue {
    format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={MAX_AGE}")
        .parse()
        .expect("valid cookie header")
}

fn clear_cookie() -> HeaderValue {
    format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
        .parse()
        .expect("valid cookie header")
}

/// (name, email) for a live session cookie, if the backend agrees.
async fn who_with_token(app: &Arc<AppState>, token: Option<&str>) -> Option<(String, String)> {
    let (token, be) = match (token, app.backend.as_ref()) {
        (Some(t), Some(b)) => (t, b),
        _ => return None,
    };
    match be.pub_fetch("/auth/me", token).await {
        Ok((200, v)) => Some((
            v.get("name")?.as_str()?.to_string(),
            v.get("email")?.as_str()?.to_string(),
        )),
        _ => None,
    }
}

fn html_page<T: Template>(page: T) -> Response {
    match page.render() {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            eprintln!("template error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn redirect(loc: &'static str, extra: Option<HeaderValue>) -> Response {
    let loc_h = HeaderValue::from_static(loc);
    match extra {
        Some(cookie) => (
            StatusCode::SEE_OTHER,
            [(header::LOCATION, loc_h), (header::SET_COOKIE, cookie)],
        )
            .into_response(),
        None => (StatusCode::SEE_OTHER, [(header::LOCATION, loc_h)]).into_response(),
    }
}

pub async fn login_page(State(app): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    // Already aboard? Straight to the profile.
    if who_with_token(&app, cookie_value(&headers).as_deref()).await.is_some() {
        return redirect("/account", None);
    }
    html_page(LoginPage {
        active: "account",
        page_title: "Log in".into(),
        context_line: "Welcome back".into(),
        error: String::new(),
        backend_on: app.backend.is_some(),
    })
}

pub async fn signup_page(State(app): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if who_with_token(&app, cookie_value(&headers).as_deref()).await.is_some() {
        return redirect("/account", None);
    }
    html_page(SignupPage {
        active: "account",
        page_title: "Join the crew".into(),
        context_line: "New accounts".into(),
        error: String::new(),
        backend_on: app.backend.is_some(),
    })
}

pub async fn account(State(app): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match who_with_token(&app, cookie_value(&headers).as_deref()).await {
        Some((name, email)) => html_page(AccountPage {
            active: "account",
            page_title: "Account".into(),
            context_line: "Crew accounts".into(),
            initial: name.chars().next().unwrap_or('F').to_ascii_uppercase(),
            name,
            email,
        }),
        None => redirect("/login", None),
    }
}

async fn handle_form(app: &Arc<AppState>, path: &str, form: AuthForm, is_signup: bool) -> Response {
    let backend_on = app.backend.is_some();
    let Some(be) = app.backend.as_ref() else {
        let msg = "Accounts need the research backend connected (API_AUTH_TOKEN in web/.env).";
        return if is_signup {
            html_page(SignupPage {
                active: "account",
                page_title: "Join the crew".into(),
                context_line: "New accounts".into(),
                error: msg.into(),
                backend_on,
            })
        } else {
            html_page(LoginPage {
                active: "account",
                page_title: "Log in".into(),
                context_line: "Welcome back".into(),
                error: msg.into(),
                backend_on,
            })
        };
    };

    let mut body = json!({ "email": form.email, "password": form.password });
    if let Some(n) = form.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        body["name"] = json!(n);
    }

    match be.pub_call(path, body, None).await {
        Err(e) => {
            eprintln!("account call failed: {e}");
            let msg = "Couldn't reach the research backend.";
            if is_signup {
                html_page(SignupPage {
                    active: "account",
                    page_title: "Join the crew".into(),
                    context_line: "New accounts".into(),
                    error: msg.into(),
                    backend_on,
                })
            } else {
                html_page(LoginPage {
                    active: "account",
                    page_title: "Log in".into(),
                    context_line: "Welcome back".into(),
                    error: msg.into(),
                    backend_on,
                })
            }
        }
        Ok((status, v)) if status == 200 || status == 201 => match v
            .get("token")
            .and_then(|t| t.as_str())
            .map(str::to_string)
        {
            Some(token) => redirect("/account", Some(cookie_header(&token))),
            None => {
                let msg = "The backend answered without a session.";
                if is_signup {
                    html_page(SignupPage {
                        active: "account",
                        page_title: "Join the crew".into(),
                        context_line: "New accounts".into(),
                        error: msg.into(),
                        backend_on,
                    })
                } else {
                    html_page(LoginPage {
                        active: "account",
                        page_title: "Log in".into(),
                        context_line: "Welcome back".into(),
                        error: msg.into(),
                        backend_on,
                    })
                }
            }
        },
        Ok((_, v)) => {
            let why = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("That didn’t work. Check the email and password.")
                .to_string();
            if is_signup {
                html_page(SignupPage {
                    active: "account",
                    page_title: "Join the crew".into(),
                    context_line: "New accounts".into(),
                    error: why,
                    backend_on,
                })
            } else {
                html_page(LoginPage {
                    active: "account",
                    page_title: "Log in".into(),
                    context_line: "Welcome back".into(),
                    error: why,
                    backend_on,
                })
            }
        }
    }
}

pub async fn signup(State(app): State<Arc<AppState>>, form: Form<AuthForm>) -> Response {
    handle_form(&app, "/auth/signup", form.0, true).await
}

pub async fn login(State(app): State<Arc<AppState>>, form: Form<AuthForm>) -> Response {
    handle_form(&app, "/auth/login", form.0, false).await
}

pub async fn logout(State(app): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let (Some(token), Some(be)) = (cookie_value(&headers), app.backend.as_ref()) {
        let _ = be.pub_call("/auth/logout", json!({}), Some(&token)).await;
    }
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, HeaderValue::from_static("/")),
            (header::SET_COOKIE, clear_cookie()),
        ],
    )
        .into_response()
}
