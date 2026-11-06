//! Browser PKCE 授权流：本地回环回调、state 校验与授权码换 token。

use std::collections::BTreeMap;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};

use super::super::connect::OAuthCredential;
use super::super::presets::PresetProviderSpec;
use super::{
    OAUTH_TIMEOUT, OAuthPromptMode, OAuthStarted, credential_from_token_for, expand_flow_url,
    http_client, json_i64, json_string, now_ms, pkce_challenge, random_token,
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_browser_flow(
    spec: &PresetProviderSpec,
    method_id: &str,
    authorize_url: &str,
    token_url: &str,
    client_id: &str,
    scope: &str,
    fixed_port: Option<u16>,
    form_answers: &BTreeMap<String, String>,
    attempt_id: String,
    cancel_rx: watch::Receiver<bool>,
    result_tx: oneshot::Sender<Result<OAuthCredential, String>>,
) -> Result<OAuthStarted, String> {
    let listener = bind_listener(fixed_port).await?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("OAuth callback listener has no address: {error}"))?
        .port();
    let callback_path = callback_path(spec.id);
    // 固定端口的供应商（OpenAI 1455 / DigitalOcean 19845）要求 localhost 白名单，
    // 监听本身仍只绑定 127.0.0.1。
    let redirect = if fixed_port.is_some() {
        format!("http://localhost:{port}{callback_path}")
    } else {
        format!("http://127.0.0.1:{port}{callback_path}")
    };
    let verifier = random_token(32)?;
    let challenge = pkce_challenge(&verifier);
    let state = random_token(24)?;
    let authorize = expand_flow_url(authorize_url, form_answers);
    let token = expand_flow_url(token_url, form_answers);
    let url = browser_authorize_url(
        spec.id, &authorize, client_id, scope, &redirect, &state, &challenge,
    );
    let started = OAuthStarted {
        attempt_id,
        url,
        user_code: None,
        instructions:
            "Complete authorization in your browser. This window will close automatically."
                .to_string(),
        mode: OAuthPromptMode::Auto,
    };
    let provider_id = spec.id.to_string();
    let method_id = method_id.to_string();
    let client_id = client_id.to_string();
    let answers = form_answers.clone();
    tokio::spawn(async move {
        let result = run_browser_flow(BrowserFlow {
            listener,
            provider_id,
            method_id,
            token_url: token,
            client_id,
            redirect,
            state,
            verifier,
            callback_path,
            answers,
            cancel_rx,
        })
        .await;
        let _ = result_tx.send(result);
    });
    Ok(started)
}

pub(super) struct BrowserFlow {
    listener: TcpListener,
    provider_id: String,
    method_id: String,
    token_url: String,
    client_id: String,
    redirect: String,
    state: String,
    verifier: String,
    callback_path: &'static str,
    answers: BTreeMap<String, String>,
    cancel_rx: watch::Receiver<bool>,
}

impl BrowserFlow {
    #[cfg(test)]
    pub(super) fn for_test(
        listener: TcpListener,
        provider_id: &str,
        token_url: &str,
        state: &str,
        callback_path: &'static str,
        cancel_rx: watch::Receiver<bool>,
    ) -> Self {
        Self {
            listener,
            provider_id: provider_id.to_string(),
            method_id: "browser".to_string(),
            token_url: token_url.to_string(),
            client_id: "client".to_string(),
            redirect: "http://127.0.0.1/auth/callback".to_string(),
            state: state.to_string(),
            verifier: "verifier".to_string(),
            callback_path,
            answers: BTreeMap::new(),
            cancel_rx,
        }
    }
}

pub(super) async fn run_browser_flow(mut flow: BrowserFlow) -> Result<OAuthCredential, String> {
    let callback = wait_for_callback(&mut flow).await?;
    let client = http_client()?;
    match callback {
        BrowserCallback::Code(code) => {
            let mut credential = exchange_authorization_code(
                &client,
                &flow.provider_id,
                &flow.method_id,
                &flow.token_url,
                &flow.client_id,
                &flow.redirect,
                &code,
                &flow.verifier,
            )
            .await?;
            if flow.provider_id == "gitlab" {
                let instance = flow
                    .answers
                    .get("instanceUrl")
                    .map(String::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("https://gitlab.com")
                    .trim_end_matches('/')
                    .to_string();
                credential
                    .metadata
                    .insert("instanceUrl".to_string(), instance);
            }
            Ok(credential)
        }
        BrowserCallback::DigitalOcean {
            access_token,
            expires_in,
        } => Ok(OAuthCredential {
            method_id: flow.method_id.clone(),
            access_token,
            refresh_token: None,
            expires_at_ms: Some(now_ms() + expires_in * 1000),
            account_label: None,
            metadata: BTreeMap::new(),
        }),
    }
}

enum BrowserCallback {
    Code(String),
    DigitalOcean {
        access_token: String,
        expires_in: i64,
    },
}

async fn wait_for_callback(flow: &mut BrowserFlow) -> Result<BrowserCallback, String> {
    let deadline = tokio::time::Instant::now() + OAUTH_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("OAuth authorization timed out".to_string());
        }
        let accepted = tokio::select! {
            result = tokio::time::timeout(remaining, flow.listener.accept()) => result,
            _ = flow.cancel_rx.changed() => return Err("OAuth flow was cancelled".to_string()),
        };
        let (mut stream, _) = accepted
            .map_err(|_| "OAuth authorization timed out".to_string())?
            .map_err(|error| format!("OAuth callback accept failed: {error}"))?;
        let request = match read_http_request(&mut stream).await {
            Ok(request) => request,
            Err(_) => {
                write_response(&mut stream, "400 Bad Request", "text/plain", "Bad request").await;
                continue;
            }
        };
        let path = request.path.as_str();
        if request.method == "POST" && path == "/auth/token" {
            match parse_digitalocean_token(&request.body, &flow.state) {
                Ok((access_token, expires_in)) => {
                    write_response(
                        &mut stream,
                        "200 OK",
                        "text/plain",
                        "Authorization complete. You can close this window.",
                    )
                    .await;
                    return Ok(BrowserCallback::DigitalOcean {
                        access_token,
                        expires_in,
                    });
                }
                Err(error) => {
                    write_response(
                        &mut stream,
                        "400 Bad Request",
                        "text/html",
                        &error_page(&error),
                    )
                    .await;
                    return Err(error);
                }
            }
        }
        if path != flow.callback_path {
            write_response(&mut stream, "404 Not Found", "text/plain", "Not found").await;
            continue;
        }
        let params = parse_query(&request.query);
        if let Some(error) = params.get("error") {
            let detail = params
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| error.clone());
            write_response(
                &mut stream,
                "400 Bad Request",
                "text/html",
                &error_page(&detail),
            )
            .await;
            return Err(detail);
        }
        // 隐式授权流（DigitalOcean）的 token/state 位于 URL fragment，首次 GET
        // 的 query 为空；state 由随后的 POST /auth/token 校验。
        let implicit_callback = flow.token_url.is_empty() || flow.provider_id == "digitalocean";
        if !implicit_callback
            && params.get("state").map(String::as_str) != Some(flow.state.as_str())
        {
            let message = "Invalid OAuth state";
            write_response(
                &mut stream,
                "400 Bad Request",
                "text/html",
                &error_page(message),
            )
            .await;
            return Err(message.to_string());
        }
        if implicit_callback {
            write_response(
                &mut stream,
                "200 OK",
                "text/html",
                &digitalocean_bootstrap_page(),
            )
            .await;
            continue;
        }
        let Some(code) = params.get("code").filter(|code| !code.is_empty()) else {
            let message = "Missing authorization code";
            write_response(
                &mut stream,
                "400 Bad Request",
                "text/html",
                &error_page(message),
            )
            .await;
            return Err(message.to_string());
        };
        write_response(
            &mut stream,
            "200 OK",
            "text/html",
            &success_page(if flow.provider_id == "openai" {
                "ChatGPT"
            } else {
                "OAuth"
            }),
        )
        .await;
        return Ok(BrowserCallback::Code(code.clone()));
    }
}

async fn exchange_authorization_code(
    client: &reqwest::Client,
    provider_id: &str,
    method_id: &str,
    token_url: &str,
    client_id: &str,
    redirect: &str,
    code: &str,
    verifier: &str,
) -> Result<OAuthCredential, String> {
    let response = client
        .post(token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|error| format!("token exchange failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("token response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!("token exchange failed: HTTP {}", status.as_u16()));
    }
    let value: Value =
        serde_json::from_str(&body).map_err(|_| "token response is invalid JSON".to_string())?;
    credential_from_token_for(provider_id, method_id, &value)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_code_flow(
    provider_id: &str,
    method_id: &str,
    authorize_url: &str,
    token_url: &str,
    client_id: &str,
    scope: &str,
    form_answers: &BTreeMap<String, String>,
    attempt_id: String,
    cancel_rx: watch::Receiver<bool>,
    mut code_rx: mpsc::Receiver<String>,
    result_tx: oneshot::Sender<Result<OAuthCredential, String>>,
) -> Result<OAuthStarted, String> {
    let state = random_token(24)?;
    let authorize = expand_flow_url(authorize_url, form_answers);
    let token = expand_flow_url(token_url, form_answers);
    let url = append_query(
        &authorize,
        &[
            ("response_type", "code"),
            ("client_id", client_id),
            ("scope", scope),
            ("state", &state),
        ],
    );
    let started = OAuthStarted {
        attempt_id,
        url,
        user_code: None,
        instructions: "Paste the authorization code shown in your browser.".to_string(),
        mode: OAuthPromptMode::Code,
    };
    let provider_id = provider_id.to_string();
    let method_id = method_id.to_string();
    let client_id = client_id.to_string();
    let token_url = token;
    let mut cancel_rx = cancel_rx;
    tokio::spawn(async move {
        let result = tokio::select! {
            code = code_rx.recv() => match code {
                Some(code) => match http_client() {
                    Ok(client) => exchange_authorization_code(
                        &client,
                        &provider_id,
                        &method_id,
                        &token_url,
                        &client_id,
                        "http://127.0.0.1/",
                        &code,
                        "",
                    ).await,
                    Err(error) => Err(error),
                },
                None => Err("OAuth flow ended without a code".to_string()),
            },
            _ = cancel_rx.changed() => Err("OAuth flow was cancelled".to_string()),
            _ = tokio::time::sleep(OAUTH_TIMEOUT) => Err("OAuth authorization timed out".to_string()),
        };
        let _ = result_tx.send(result);
    });
    Ok(started)
}

async fn bind_listener(fixed_port: Option<u16>) -> Result<TcpListener, String> {
    let port = fixed_port.unwrap_or(0);
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| {
            format!("OAuth callback listener could not bind 127.0.0.1:{port}: {error}")
        })
}

fn callback_path(provider_id: &str) -> &'static str {
    match provider_id {
        "gitlab" | "poe" => "/callback",
        _ => "/auth/callback",
    }
}

pub(super) fn browser_authorize_url(
    provider_id: &str,
    authorize: &str,
    client_id: &str,
    scope: &str,
    redirect: &str,
    state: &str,
    challenge: &str,
) -> String {
    if provider_id == "digitalocean" {
        return append_query(
            authorize,
            &[
                ("response_type", "token"),
                ("client_id", client_id),
                ("redirect_uri", redirect),
                ("scope", scope),
                ("state", state),
            ],
        );
    }
    let mut params = vec![
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("scope", scope),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    if provider_id == "openai" {
        params.push(("id_token_add_organizations", "true"));
        params.push(("codex_cli_simplified_flow", "true"));
        params.push(("originator", "opencode"));
    }
    append_query(authorize, &params)
}

fn append_query(base: &str, params: &[(&str, &str)]) -> String {
    let encoded = params
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<String>>()
        .join("&");
    if encoded.is_empty() {
        return base.to_string();
    }
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}{encoded}")
}

struct HttpRequest {
    method: String,
    path: String,
    query: String,
    body: String,
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read callback request: {error}"))?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 16 * 1024 {
            return Err("callback request is too large".to_string());
        }
    }
    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "callback request has no headers".to_string())?;
    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = headers.lines();
    let request_line = lines
        .next()
        .ok_or_else(|| "callback request has no request line".to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| "callback request has no method".to_string())?
        .to_string();
    let target = parts
        .next()
        .ok_or_else(|| "callback request has no target".to_string())?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (target.to_string(), String::new()),
    };
    let content_length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|error| format!("read callback body: {error}"))?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        query,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

async fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub(super) fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(key), percent_decode(value));
    }
    out
}

fn parse_digitalocean_token(body: &str, expected_state: &str) -> Result<(String, i64), String> {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let state = json_string(&value, "state")
        .or_else(|| parse_query(body).get("state").cloned())
        .unwrap_or_default();
    if state != expected_state {
        return Err("Invalid OAuth state".to_string());
    }
    let access_token = json_string(&value, "access_token")
        .or_else(|| parse_query(body).get("access_token").cloned())
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "Missing access token".to_string())?;
    let expires_in = json_i64(&value, "expires_in")
        .or_else(|| {
            parse_query(body)
                .get("expires_in")
                .and_then(|value| value.parse::<i64>().ok())
        })
        .filter(|seconds| *seconds > 0)
        .unwrap_or(60 * 60 * 24 * 30);
    Ok((access_token, expires_in))
}

pub(super) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                match (high, low) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        index += 3;
                    }
                    _ => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(super) fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

/// 浏览器回调页模板（`{{TITLE}}` / `{{BODY}}` 占位）。
const CALLBACK_PAGE: &str = include_str!("../../../../assets/oauth_callback.html");
/// DigitalOcean 隐式授权：浏览器从 URL fragment 取 token 并 POST 回本地。
const IMPLICIT_PAGE: &str = include_str!("../../../../assets/oauth_implicit.html");

fn success_page(provider: &str) -> String {
    let body = format!(
        "<h1>Authorization complete</h1><p>{} is now connected to lx. You can close this window.</p>",
        escape_html(provider)
    );
    callback_page("Authorization complete", &body)
}

fn error_page(message: &str) -> String {
    let body = format!(
        "<h1>Authorization failed</h1><p>{}</p>",
        escape_html(message)
    );
    callback_page("Authorization failed", &body)
}

fn callback_page(title: &str, body: &str) -> String {
    CALLBACK_PAGE
        .replace("{{TITLE}}", &escape_html(title))
        .replace("{{BODY}}", body)
}

fn digitalocean_bootstrap_page() -> String {
    IMPLICIT_PAGE.to_string()
}
