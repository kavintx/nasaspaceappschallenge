//! Running as a serverless function (Netlify Functions, which are AWS Lambda underneath).
//!
//! There is no port to listen on there. Lambda hands the program one request at a time
//! over its Runtime API: we ask for the next event, turn it into an ordinary HTTP request,
//! give that to the same router the normal server uses, and post the reply back.
//! main.rs switches to this when AWS_LAMBDA_RUNTIME_API is set, which Lambda always does.

use axum::body::Body;
use axum::http::{HeaderName, HeaderValue, Method, Request};
use axum::Router;
use base64::Engine;
use serde_json::{json, Map, Value};
use tower::ServiceExt;

/// Largest reply we will buffer. Lambda itself refuses replies over 6 MB.
const MAX_REPLY_BYTES: usize = 6 * 1024 * 1024;

pub async fn run(app: Router, runtime_api: String) {
    let client = reqwest::Client::new();
    let base = format!("http://{runtime_api}/2018-06-01/runtime/invocation");
    loop {
        let next = match client.get(format!("{base}/next")).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("lambda: could not fetch the next event: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
        };
        let Some(id) = next.headers().get("lambda-runtime-aws-request-id").and_then(|v| v.to_str().ok()).map(str::to_string) else {
            continue;
        };
        let reply = match next.json::<Value>().await {
            Ok(event) => answer(&app, &event).await,
            Err(e) => Err(format!("unreadable event: {e}")),
        };
        let sent = match reply {
            Ok(body) => client.post(format!("{base}/{id}/response")).json(&body).send().await,
            Err(msg) => {
                eprintln!("lambda: {msg}");
                client
                    .post(format!("{base}/{id}/error"))
                    .json(&json!({ "errorMessage": msg, "errorType": "RequestError" }))
                    .send()
                    .await
            }
        };
        if let Err(e) = sent {
            eprintln!("lambda: could not send the reply: {e}");
        }
    }
}

/// One event in, one reply out. Bodies travel as base64 so pictures and gzip survive.
async fn answer(app: &Router, event: &Value) -> Result<Value, String> {
    let request = to_request(event)?;
    // The router cannot fail: its error type is Infallible.
    let Ok(response) = app.clone().oneshot(request).await;
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, MAX_REPLY_BYTES).await.map_err(|e| format!("reply too large or broken: {e}"))?;

    let mut headers: Map<String, Value> = Map::new();
    for (name, value) in parts.headers.iter() {
        // The body is sent whole, so these two no longer describe it.
        if name == "transfer-encoding" || name == "content-length" {
            continue;
        }
        let Ok(text) = value.to_str() else { continue };
        if let Value::Array(list) = headers.entry(name.as_str()).or_insert_with(|| Value::Array(Vec::new())) {
            list.push(Value::String(text.to_string()));
        }
    }
    Ok(json!({
        "statusCode": parts.status.as_u16(),
        "multiValueHeaders": headers,
        "body": base64::engine::general_purpose::STANDARD.encode(&bytes),
        "isBase64Encoded": true,
    }))
}

fn to_request(event: &Value) -> Result<Request<Body>, String> {
    let method = event["httpMethod"].as_str().unwrap_or("GET");
    let method = Method::from_bytes(method.as_bytes()).map_err(|_| "bad method".to_string())?;
    let path = request_path(event);
    let query = query_string(event);
    let uri = if query.is_empty() { path.to_string() } else { format!("{path}?{query}") };

    let mut request = Request::builder().method(method).uri(uri);
    let mut add = |name: &str, value: &str| {
        if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            request.headers_mut().map(|h| h.append(n, v));
        }
    };
    match event["multiValueHeaders"].as_object().filter(|m| !m.is_empty()) {
        Some(many) => {
            for (name, values) in many {
                for v in values.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    add(name, v);
                }
            }
        }
        None => {
            for (name, v) in event["headers"].as_object().into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    add(name, v);
                }
            }
        }
    }

    let raw = event["body"].as_str().unwrap_or("");
    let body = if event["isBase64Encoded"].as_bool().unwrap_or(false) {
        base64::engine::general_purpose::STANDARD.decode(raw).map_err(|_| "body is not valid base64".to_string())?
    } else {
        raw.as_bytes().to_vec()
    };
    request.body(Body::from(body)).map_err(|e| e.to_string())
}

/// The path the visitor asked for. Every page is rewritten to this one function, and hosts
/// differ on whether `path` then holds the page or the function's own address, so the full
/// address (`rawUrl`) is preferred when it is there.
fn request_path(event: &Value) -> String {
    let from_url = event["rawUrl"].as_str().and_then(|url| {
        let rest = url.split_once("://")?.1;
        let path = &rest[rest.find('/')?..];
        Some(path.split(['?', '#']).next().unwrap_or("/").to_string())
    });
    let path = from_url.or_else(|| event["path"].as_str().map(str::to_string)).unwrap_or_default();
    let path = match path.strip_prefix("/.netlify/functions/") {
        Some(rest) => rest.find('/').map(|i| rest[i..].to_string()).unwrap_or_default(),
        None => path,
    };
    if path.starts_with('/') { path } else { "/".to_string() }
}

/// The query exactly as sent when the platform gives it; otherwise rebuilt from the parsed form.
fn query_string(event: &Value) -> String {
    if let Some(raw) = event["rawQuery"].as_str() {
        return raw.to_string();
    }
    let mut pairs: Vec<String> = Vec::new();
    let pair = |k: &str, v: &str| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v));
    match event["multiValueQueryStringParameters"].as_object().filter(|m| !m.is_empty()) {
        Some(many) => {
            for (k, values) in many {
                for v in values.as_array().into_iter().flatten().filter_map(Value::as_str) {
                    pairs.push(pair(k, v));
                }
            }
        }
        None => {
            for (k, v) in event["queryStringParameters"].as_object().into_iter().flatten() {
                if let Some(v) = v.as_str() {
                    pairs.push(pair(k, v));
                }
            }
        }
    }
    pairs.join("&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};

    fn app() -> Router {
        Router::new()
            .route("/hello", get(|q: axum::extract::RawQuery| async move { format!("hi {}", q.0.unwrap_or_default()) }))
            .route("/echo", post(|body: axum::body::Bytes| async move { body }))
    }

    fn body_of(reply: &Value) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.decode(reply["body"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn get_with_query() {
        let event = json!({ "httpMethod": "GET", "path": "/hello", "rawQuery": "mode=label", "headers": { "host": "example.test" } });
        let reply = answer(&app(), &event).await.unwrap();
        assert_eq!(reply["statusCode"], 200);
        assert_eq!(body_of(&reply), b"hi mode=label");
    }

    #[tokio::test]
    async fn query_rebuilt_when_raw_is_missing() {
        let event = json!({ "httpMethod": "GET", "path": "/hello", "queryStringParameters": { "a": "1 2" } });
        let reply = answer(&app(), &event).await.unwrap();
        assert_eq!(body_of(&reply), b"hi a=1%202");
    }

    #[tokio::test]
    async fn binary_post_round_trips() {
        let bytes = [0u8, 159, 146, 150, 255];
        let event = json!({ "httpMethod": "POST", "path": "/echo", "isBase64Encoded": true,
            "body": base64::engine::general_purpose::STANDARD.encode(bytes) });
        let reply = answer(&app(), &event).await.unwrap();
        assert_eq!(body_of(&reply), bytes);
    }

    #[test]
    fn path_comes_from_the_full_address_or_the_event() {
        assert_eq!(request_path(&json!({ "path": "/.netlify/functions/app", "rawUrl": "https://x.netlify.app/papers/3?a=1" })), "/papers/3");
        assert_eq!(request_path(&json!({ "path": "/scan" })), "/scan");
        assert_eq!(request_path(&json!({ "path": "/.netlify/functions/app/items" })), "/items");
        assert_eq!(request_path(&json!({ "path": "/.netlify/functions/app" })), "/");
        assert_eq!(request_path(&json!({ "rawUrl": "https://x.netlify.app" })), "/");
    }

    #[tokio::test]
    async fn unknown_path_is_404() {
        let reply = answer(&app(), &json!({ "httpMethod": "GET", "path": "/nope" })).await.unwrap();
        assert_eq!(reply["statusCode"], 404);
    }
}
