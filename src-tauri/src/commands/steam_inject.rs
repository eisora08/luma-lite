use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const CEF_DEBUG_PORTS: &[u16] = &[8080, 8081, 8082, 8083, 9222];
const HTTP_TIMEOUT_SECS: u64 = 3;
const WS_RETRY_ATTEMPTS: u32 = 2;
const LUMA_INJECT_VERSION: &str = "2.5.0-download-flow";
const TARGET_MONITOR_INTERVAL_MS: u64 = 1500;

const CDP_ID_PAGE_ENABLE: u64 = 1;
const CDP_ID_RUNTIME_ENABLE: u64 = 2;
const CDP_ID_BYPASS_CSP: u64 = 3;
const CDP_ID_INJECT_SCRIPT: u64 = 4;
const CDP_ID_NETWORK_DIAGNOSTIC: u64 = 5;
const CDP_ID_ADD_SCRIPT_ON_NEW_DOC: u64 = 6;
const CDP_ID_LIFECYCLE_VERIFY: u64 = 7;

const CDP_ID_PAGE_RELOAD: u64 = 8;
const CDP_ID_POST_RELOAD_VERIFY: u64 = 9;

#[derive(Debug, Deserialize)]
struct CefTab {
    id: Option<String>,
    url: Option<String>,
    title: Option<String>,

    #[serde(rename = "webSocketDebuggerUrl")]
    web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectResult {
    pub success: bool,
    pub tab_title: Option<String>,
    pub tab_url: Option<String>,
    pub error: Option<String>,
    pub tabs_found: usize,
    pub tabs_matched: usize,
    pub injected_tab_urls: Vec<String>,
    pub injected_target_ids: Vec<String>,
    pub tabs_already_injected: usize,
    pub debug_port: u16,
    pub bridge_reachable: Option<bool>,
    pub bridge_error: Option<String>,
}

struct InjectedTarget {
    target_id: String,
    last_url: String,
    ws_url: Option<String>,
    script_identifier: Option<String>,
    version: String,
    last_verified: Option<Instant>,
}

struct InjectedExtension {
    target_url: String,
    injected_targets: Vec<InjectedTarget>,
}

static INJECTED: OnceLock<DashMap<String, InjectedExtension>> = OnceLock::new();

fn get_injected() -> &'static DashMap<String, InjectedExtension> {
    INJECTED.get_or_init(DashMap::new)
}

pub fn track_injection(
    extension_id: &str,
    target_url: &str,
    target_ids: &[String],
    urls: &[String],
    ws_urls: &[Option<String>],
    script_ids: &[Option<String>],
) {
    let targets = target_ids
        .iter()
        .zip(urls.iter())
        .enumerate()
        .map(|(i, (id, url))| InjectedTarget {
            target_id: id.clone(),
            last_url: url.clone(),
            ws_url: ws_urls.get(i).cloned().flatten(),
            script_identifier: script_ids.get(i).cloned().flatten(),
            version: LUMA_INJECT_VERSION.to_string(),
            last_verified: None,
        })
        .collect();

    get_injected().insert(
        extension_id.to_string(),
        InjectedExtension {
            target_url: target_url.to_string(),
            injected_targets: targets,
        },
    );
}

pub fn clear_injection(extension_id: &str) {
    get_injected().remove(extension_id);
}

pub fn get_injected_target_ids(extension_id: &str) -> Vec<String> {
    get_injected()
        .get(extension_id)
        .map(|entry| {
            entry
                .injected_targets
                .iter()
                .map(|t| t.target_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn detect_cef_debug_port() -> Option<u16> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!("[CEF_INJECT] Failed to create CEF detection client: {error}");
            return None;
        }
    };

    for &port in CEF_DEBUG_PORTS {
        let url = format!("http://127.0.0.1:{port}/json");

        match client.get(&url).send() {
            Ok(response) if response.status().is_success() => {
                eprintln!("[CEF_INJECT] Detected CEF debugger on port {port}");
                return Some(port);
            }
            Ok(response) => {
                eprintln!(
                    "[CEF_INJECT] Port {port} responded with status {}",
                    response.status()
                );
            }
            Err(_) => {}
        }
    }

    eprintln!(
        "[CEF_INJECT] No CEF debugger found on common ports: {}",
        CEF_DEBUG_PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );

    None
}

fn send_cdp_command(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    command: &Value,
    expected_id: u64,
    command_name: &str,
) -> Result<Value, String> {
    let serialized = serde_json::to_string(command)
        .map_err(|error| format!("{command_name} serialization failed: {error}"))?;

    socket
        .send(Message::Text(serialized))
        .map_err(|error| format!("{command_name} send failed: {error}"))?;

    loop {
        let message = socket
            .read()
            .map_err(|error| format!("{command_name} read failed: {error}"))?;

        match message {
            Message::Text(text) => {
                let parsed: Value = match serde_json::from_str(&text) {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!(
                            "[CEF_INJECT] Ignoring non-JSON CDP text frame while waiting for {}: {}",
                            command_name, error
                        );
                        continue;
                    }
                };

                if let Some(method) = parsed.get("method").and_then(Value::as_str) {
                    if method == "Runtime.exceptionThrown" {
                        eprintln!("[CEF_INJECT] Runtime exception event: {}", parsed);
                    } else if method == "Runtime.consoleAPICalled" {
                        eprintln!("[CEF_CONSOLE] {}", format_console_event(&parsed));
                    }

                    continue;
                }

                let response_id = parsed.get("id").and_then(Value::as_u64);

                if response_id != Some(expected_id) {
                    continue;
                }

                if let Some(error) = parsed.get("error") {
                    let code = error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default();

                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Unknown CDP error");

                    return Err(format!(
                        "{command_name} returned CDP error {code}: {message}"
                    ));
                }

                if let Some(exception) = parsed.pointer("/result/exceptionDetails") {
                    return Err(format_exception_details(command_name, exception));
                }

                return Ok(parsed);
            }

            Message::Ping(payload) => {
                socket.send(Message::Pong(payload)).map_err(|error| {
                    format!("{command_name} failed to answer WebSocket ping: {error}")
                })?;
            }

            Message::Close(frame) => {
                return Err(format!(
                    "{command_name} failed because CDP closed the WebSocket: {frame:?}"
                ));
            }

            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}



fn send_page_reload_and_wait(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<Value, String> {
    let command = json!({
        "id": CDP_ID_PAGE_RELOAD,
        "method": "Page.reload",
        "params": {
            "ignoreCache": true
        }
    });

    let serialized = serde_json::to_string(&command)
        .map_err(|error| {
            format!("Page.reload serialization failed: {error}")
        })?;

    socket
        .send(Message::Text(serialized))
        .map_err(|error| {
            format!("Page.reload send failed: {error}")
        })?;

    let mut reload_response: Option<Value> = None;
    let mut load_event_received = false;

    while reload_response.is_none() || !load_event_received {
        let message = socket
            .read()
            .map_err(|error| {
                format!(
                    "Waiting for Page.reload completion failed: {error}"
                )
            })?;

        match message {
            Message::Text(text) => {
                let parsed: Value =
                    match serde_json::from_str(&text) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };

                if parsed
                    .get("method")
                    .and_then(Value::as_str)
                    == Some("Page.loadEventFired")
                {
                    load_event_received = true;

                    eprintln!(
                        "[CEF_INJECT] Page.loadEventFired received"
                    );

                    continue;
                }

                if let Some(method) =
                    parsed.get("method").and_then(Value::as_str)
                {
                    if method == "Runtime.exceptionThrown" {
                        eprintln!(
                            "[CEF_INJECT] Runtime exception event: {}",
                            parsed
                        );
                    } else if method == "Runtime.consoleAPICalled" {
                        eprintln!(
                            "[CEF_CONSOLE] {}",
                            format_console_event(&parsed)
                        );
                    }

                    continue;
                }

                if parsed.get("id").and_then(Value::as_u64)
                    != Some(CDP_ID_PAGE_RELOAD)
                {
                    continue;
                }

                if let Some(error) = parsed.get("error") {
                    return Err(format!(
                        "Page.reload returned a CDP error: {error}"
                    ));
                }

                reload_response = Some(parsed);
            }

            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .map_err(|error| {
                        format!(
                            "Failed to answer WebSocket ping during reload: {error}"
                        )
                    })?;
            }

            Message::Close(frame) => {
                return Err(format!(
                    "CDP closed during Page.reload: {frame:?}"
                ));
            }

            Message::Binary(_)
            | Message::Pong(_)
            | Message::Frame(_) => {}
        }
    }

    reload_response.ok_or_else(|| {
        "Page.reload completed without a command response."
            .to_string()
    })
}


fn format_exception_details(command_name: &str, exception: &Value) -> String {
    let text = exception
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("JavaScript exception");

    let line = exception
        .get("lineNumber")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let column = exception
        .get("columnNumber")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let description = exception
        .pointer("/exception/description")
        .and_then(Value::as_str)
        .unwrap_or("");

    let stack = exception
        .pointer("/stackTrace/callFrames")
        .cloned()
        .unwrap_or(Value::Null);

    format!(
        "{command_name} JavaScript exception: {text}; description={description}; line={line}; column={column}; stack={stack}"
    )
}

fn format_console_event(event: &Value) -> String {
    let console_type = event
        .pointer("/params/type")
        .and_then(Value::as_str)
        .unwrap_or("log");

    let arguments = event
        .pointer("/params/args")
        .and_then(Value::as_array)
        .map(|args| {
            args.iter()
                .map(|argument| {
                    argument
                        .get("value")
                        .map(Value::to_string)
                        .or_else(|| {
                            argument
                                .get("description")
                                .and_then(Value::as_str)
                                .map(ToString::to_string)
                        })
                        .unwrap_or_else(|| argument.to_string())
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    format!("type={console_type} args={arguments}")
}

fn extract_returned_value(response: &Value) -> Option<&Value> {
    response.pointer("/result/result/value")
}


fn verify_lifecycle_on_socket(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    command_id: u64,
    label: &str,
) -> Result<bool, String> {
    let expression = r##"
(function() {
    var ns = window.__lumaforge_ssh__;

    if (!ns) {
        return {
            exists: false,
            active: false
        };
    }

    return {
        exists: true,
        version: ns.version || "unknown",
        documentId: ns.documentId || "unknown",
        active: !!ns.active,
        currentAppId: ns.currentAppId || null,
        reconcileCount: ns.reconcileCount || 0,
        observerActive: !!ns.observerActive,
        historyWrapped: !!ns.historyWrapped,
        buttonCount: document.querySelectorAll(
            "#luma-action-btn"
        ).length
    };
})()
"##;

    let command = json!({
        "id": command_id,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "returnByValue": true
        }
    });

    let response = send_cdp_command(
        socket,
        &command,
        command_id,
        label,
    )?;

    eprintln!(
        "[CEF_VERIFY] {label} response: {response}"
    );

    let value = response
        .pointer("/result/result/value")
        .ok_or_else(|| {
            format!(
                "{label} returned no lifecycle value: {response}"
            )
        })?;

    let exists = value
        .get("exists")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let active = value
        .get("active")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let version = value
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("none");

    let document_id = value
        .get("documentId")
        .and_then(Value::as_str)
        .unwrap_or("none");

    let app_id = value
        .get("currentAppId")
        .and_then(Value::as_str)
        .unwrap_or("none");

    let button_count = value
        .get("buttonCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    eprintln!("[CEF_VERIFY] Lifecycle exists: {exists}");
    eprintln!("[CEF_VERIFY] Version: {version}");
    eprintln!("[CEF_VERIFY] Document ID: {document_id}");
    eprintln!("[CEF_VERIFY] Active: {active}");
    eprintln!("[CEF_VERIFY] Current AppID: {app_id}");
    eprintln!("[CEF_VERIFY] Button count: {button_count}");

    Ok(exists && active)
}
fn run_network_diagnostic(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<(), String> {
    let expression = r#"
(async () => {
    const url = "http://127.0.0.1:21775/api/providers";

    try {
        const response = await fetch(url, {
            method: "GET",
            mode: "cors",
            cache: "no-store"
        });

        const body = await response.text();

        return {
            reached: true,
            status: response.status,
            ok: response.ok,
            type: response.type,
            responseUrl: response.url,
            body: body,
            href: window.location.href,
            origin: window.location.origin,
            secureContext: window.isSecureContext
        };
    } catch (error) {
        return {
            reached: false,
            name: error && error.name ? error.name : null,
            message: error && error.message ? error.message : null,
            stack: error && error.stack ? error.stack : null,
            href: window.location.href,
            origin: window.location.origin,
            secureContext: window.isSecureContext
        };
    }
})()
"#;

    let command = json!({
        "id": CDP_ID_NETWORK_DIAGNOSTIC,
        "method": "Runtime.evaluate",
        "params": {
            "expression": expression,
            "awaitPromise": true,
            "returnByValue": true
        }
    });

    let response = send_cdp_command(
        socket,
        &command,
        CDP_ID_NETWORK_DIAGNOSTIC,
        "Network diagnostic",
    )?;

    eprintln!("[CDP_DIAG] Runtime.evaluate response: {}", response);

    let value = extract_returned_value(&response)
        .ok_or_else(|| format!("Network diagnostic returned no value: {}", response))?;

    let reached = value
        .get("reached")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !reached {
        let name = value.get("name").and_then(Value::as_str).unwrap_or("Error");

        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Unknown fetch error");

        let href = value.get("href").and_then(Value::as_str).unwrap_or("");

        let origin = value.get("origin").and_then(Value::as_str).unwrap_or("");

        let secure_context = value
            .get("secureContext")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        return Err(format!(
            "Fetch failed inside Steam CEF: {name}: {message}; href={href}; origin={origin}; secureContext={secure_context}"
        ));
    }

    let status = value
        .get("status")
        .and_then(Value::as_u64)
        .unwrap_or_default();

    let ok = value.get("ok").and_then(Value::as_bool).unwrap_or(false);

    let response_url = value
        .get("responseUrl")
        .and_then(Value::as_str)
        .unwrap_or("");

    let body = value.get("body").and_then(Value::as_str).unwrap_or("");

    eprintln!(
        "[CDP_DIAG] Provider fetch reached bridge: status={status}, ok={ok}, response_url={response_url}, body={body}"
    );

    if !ok {
        return Err(format!("Provider endpoint returned HTTP {status}: {body}"));
    }

    Ok(())
}

struct ConnectReport {
    lifecycle_ok: bool,
    bridge_ok: bool,
    lifecycle_error: Option<String>,
    bridge_error: Option<String>,
}

fn connect_once(
    ws_url: &str,
    js_code: &str,
    target_id: &str,
) -> ConnectReport {
    let mut report = ConnectReport {
        lifecycle_ok: false,
        bridge_ok: false,
        lifecycle_error: None,
        bridge_error: None,
    };

    let mut socket = match connect(ws_url) {
        Ok((socket, _)) => socket,
        Err(error) => {
            report.lifecycle_error =
                Some(format!("WebSocket connect failed: {error}"));

            return report;
        }
    };

    let page_enable = json!({
        "id": CDP_ID_PAGE_ENABLE,
        "method": "Page.enable"
    });

    match send_cdp_command(
        &mut socket,
        &page_enable,
        CDP_ID_PAGE_ENABLE,
        "Page.enable",
    ) {
        Ok(response) => {
            eprintln!(
                "[CEF_INJECT] Page.enable response: {}",
                response
            );
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    let runtime_enable = json!({
        "id": CDP_ID_RUNTIME_ENABLE,
        "method": "Runtime.enable"
    });

    match send_cdp_command(
        &mut socket,
        &runtime_enable,
        CDP_ID_RUNTIME_ENABLE,
        "Runtime.enable",
    ) {
        Ok(response) => {
            eprintln!(
                "[CEF_INJECT] Runtime.enable response: {}",
                response
            );
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    let bypass_csp = json!({
        "id": CDP_ID_BYPASS_CSP,
        "method": "Page.setBypassCSP",
        "params": {
            "enabled": true
        }
    });

    match send_cdp_command(
        &mut socket,
        &bypass_csp,
        CDP_ID_BYPASS_CSP,
        "Page.setBypassCSP",
    ) {
        Ok(response) => {
            eprintln!(
                "[CEF_INJECT] Page.setBypassCSP response: {}",
                response
            );
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    let add_script_on_new_document = json!({
        "id": CDP_ID_ADD_SCRIPT_ON_NEW_DOC,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": {
            "source": js_code
        }
    });

    match send_cdp_command(
        &mut socket,
        &add_script_on_new_document,
        CDP_ID_ADD_SCRIPT_ON_NEW_DOC,
        "Page.addScriptToEvaluateOnNewDocument",
    ) {
        Ok(response) => {
            eprintln!(
                "[CEF_INJECT] addScriptToEvaluateOnNewDocument response: {}",
                response
            );

            match response
                .pointer("/result/identifier")
                .and_then(Value::as_str)
            {
                Some(identifier) => {
                    eprintln!(
                        "[CEF_INJECT] New-document lifecycle registered for target: {} (identifier: {})",
                        target_id,
                        identifier
                    );
                }

                None => {
                    eprintln!(
                        "[CEF_INJECT] WARNING: New-document registration returned no identifier for target: {}",
                        target_id
                    );
                }
            }
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    let inject_command = json!({
        "id": CDP_ID_INJECT_SCRIPT,
        "method": "Runtime.evaluate",
        "params": {
            "expression": js_code,
            "awaitPromise": true,
            "returnByValue": true
        }
    });

    match send_cdp_command(
        &mut socket,
        &inject_command,
        CDP_ID_INJECT_SCRIPT,
        "Runtime.evaluate injection",
    ) {
        Ok(inject_response) => {
            eprintln!(
                "[CEF_INJECT] Runtime.evaluate response: {}",
                inject_response
            );

            if let Some(result_object) =
                inject_response.pointer("/result/result")
            {
                if result_object
                    .get("subtype")
                    .and_then(Value::as_str)
                    == Some("error")
                {
                    let description = result_object
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or(
                            "JavaScript evaluation returned an error",
                        );

                    report.lifecycle_error = Some(format!(
                        "Runtime.evaluate returned an error result: {description}"
                    ));

                    let _ = socket.close(None);

                    return report;
                }
            }
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    eprintln!(
        "[CEF_INJECT] Lifecycle evaluation completed for target: {}",
        target_id
    );

    match verify_lifecycle_on_socket(
        &mut socket,
        CDP_ID_LIFECYCLE_VERIFY,
        "Lifecycle verification before bridge test",
    ) {
        Ok(true) => {
            report.lifecycle_ok = true;

            eprintln!(
                "[CEF_INJECT] Lifecycle active before bridge test for target: {}",
                target_id
            );
        }

        Ok(false) => {
            report.lifecycle_error = Some(
                "Lifecycle did not become active after evaluation."
                    .to_string(),
            );

            let _ = socket.close(None);

            return report;
        }

        Err(error) => {
            report.lifecycle_error = Some(error);
            let _ = socket.close(None);

            return report;
        }
    }

    match run_network_diagnostic(&mut socket) {
        Ok(()) => {
            eprintln!(
                "[CEF_INJECT] Bridge connectivity verified for target: {}",
                target_id
            );

            report.bridge_ok = true;
            report.bridge_error = None;
        }

        Err(initial_bridge_error) => {
            eprintln!(
                "[CEF_INJECT] Initial bridge diagnostic failed for target {}: {}",
                target_id,
                initial_bridge_error
            );

            eprintln!(
                "[CEF_INJECT] Performing one-time Store reload for target: {}",
                target_id
            );

            match send_page_reload_and_wait(&mut socket) {
                Ok(reload_response) => {
                    eprintln!(
                        "[CEF_INJECT] Page.reload response: {}",
                        reload_response
                    );
                }

                Err(reload_error) => {
                    report.bridge_ok = false;

                    report.bridge_error = Some(format!(
                        "Initial bridge error: {initial_bridge_error}; automatic reload failed: {reload_error}"
                    ));

                    let _ = socket.close(None);

                    return report;
                }
            }

            match verify_lifecycle_on_socket(
                &mut socket,
                CDP_ID_POST_RELOAD_VERIFY,
                "Lifecycle verification after reload",
            ) {
                Ok(true) => {
                    report.lifecycle_ok = true;

                    eprintln!(
                        "[CEF_INJECT] Lifecycle active after Store reload for target: {}",
                        target_id
                    );
                }

                Ok(false) => {
                    report.lifecycle_ok = false;

                    report.lifecycle_error = Some(
                        "Lifecycle was not active after the Store reload."
                            .to_string(),
                    );

                    let _ = socket.close(None);

                    return report;
                }

                Err(error) => {
                    report.lifecycle_ok = false;
                    report.lifecycle_error = Some(error);

                    let _ = socket.close(None);

                    return report;
                }
            }

            match run_network_diagnostic(&mut socket) {
                Ok(()) => {
                    eprintln!(
                        "[CEF_INJECT] Bridge connectivity verified after reload for target: {}",
                        target_id
                    );

                    report.bridge_ok = true;
                    report.bridge_error = None;
                }

                Err(post_reload_error) => {
                    eprintln!(
                        "[CEF_INJECT] Bridge still unreachable after Store reload for target {}: {}",
                        target_id,
                        post_reload_error
                    );

                    report.bridge_ok = false;

                    report.bridge_error = Some(format!(
                        "Initial bridge error: {initial_bridge_error}; after reload: {post_reload_error}"
                    ));
                }
            }
        }
    }

    let _ = socket.close(None);

    report
}

fn inject_into_tab(
    tab: &CefTab,
    js_code: &str,
) -> (bool, Option<String>, Option<bool>, Option<String>) {
    let ws_url = match &tab.web_socket_debugger_url {
        Some(url) => url.clone(),
        None => {
            return (
                false,
                Some("No webSocketDebuggerUrl available for this tab".to_string()),
                None,
                None,
            );
        }
    };

    let tab_target_id = tab.id.clone().unwrap_or_default();
    let mut last_error = None;
    let mut last_bridge_ok = None;
    let mut last_bridge_error = None;

    for attempt in 0..=WS_RETRY_ATTEMPTS {
        if attempt > 0 {
            eprintln!(
                "[CEF_INJECT] Retry attempt {}/{} for tab WebSocket: {}",
                attempt,
                WS_RETRY_ATTEMPTS,
                truncate_url(&ws_url, 60)
            );

            std::thread::sleep(std::time::Duration::from_millis(200 * u64::from(attempt)));
        }

        let report = connect_once(&ws_url, js_code, &tab_target_id);
        last_bridge_ok = Some(report.bridge_ok);
        last_bridge_error = report.bridge_error;

        if report.lifecycle_ok {
            return (true, None, last_bridge_ok, last_bridge_error);
        }

        last_error = report.lifecycle_error;
    }

    (false, last_error, last_bridge_ok, last_bridge_error)
}

fn truncate_url(url: &str, max_len: usize) -> String {
    if url.len() <= max_len {
        url.to_string()
    } else {
        format!("{}...", &url[..max_len - 3])
    }
}

fn verify_lifecycle(ws_url: &str) -> Result<bool, String> {
    let (mut socket, _) =
        connect(ws_url).map_err(|error| format!("WebSocket connect failed: {error}"))?;

    let runtime_enable = json!({
        "id": CDP_ID_RUNTIME_ENABLE,
        "method": "Runtime.enable"
    });
    let _ = send_cdp_command(
        &mut socket,
        &runtime_enable,
        CDP_ID_RUNTIME_ENABLE,
        "Runtime.enable",
    );

    let verify_expression = r#"
(function() {
    var ns = window.__lumaforge_ssh__;
    if (!ns) return { exists: false };
    return {
        exists: true,
        version: ns.version || 'unknown',
        active: !!ns.active
    };
})()
"#;

    let verify_command = json!({
        "id": CDP_ID_LIFECYCLE_VERIFY,
        "method": "Runtime.evaluate",
        "params": {
            "expression": verify_expression,
            "returnByValue": true
        }
    });

    let response = send_cdp_command(
        &mut socket,
        &verify_command,
        CDP_ID_LIFECYCLE_VERIFY,
        "Lifecycle check",
    )?;

    let exists = response
        .pointer("/result/result/value/exists")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let active = response
        .pointer("/result/result/value/active")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let version = response
        .pointer("/result/result/value/version")
        .and_then(Value::as_str)
        .unwrap_or("none");

    eprintln!(
        "[CEF_INJECT] Lifecycle check on target: exists={}, active={}, version={}",
        exists, active, version
    );

    let _ = socket.close(None);

    Ok(exists && active)
}

pub fn inject_code_into_tabs(
    target_url: &str,
    js_code: &str,
    skip_tab_urls: &[String],
) -> InjectResult {
    let debug_port = match detect_cef_debug_port() {
        Some(port) => port,
        None => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(
                    "Cannot find Steam's CEF debugger. Make sure Steam is running with CEF debugging enabled."
                        .to_string(),
                ),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let debug_endpoint = format!("http://127.0.0.1:{debug_port}/json");

    let skip_target_ids: HashSet<String> = skip_tab_urls.iter().cloned().collect();

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!("Failed to build HTTP client: {error}")),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let response = match client.get(&debug_endpoint).send() {
        Ok(response) => response,
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Cannot reach Steam CEF debugger at {debug_endpoint}: {error}"
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let tabs: Vec<CefTab> = match response.text() {
        Ok(raw) => match serde_json::from_str::<Vec<CefTab>>(&raw) {
            Ok(tabs) => tabs,
            Err(error) => {
                return InjectResult {
                    success: false,
                    tab_title: None,
                    tab_url: None,
                    error: Some(format!("Failed to parse CEF tab list: {error}")),
                    tabs_found: 0,
                    tabs_matched: 0,
                    injected_tab_urls: Vec::new(),
                    injected_target_ids: Vec::new(),
                    tabs_already_injected: 0,
                    debug_port,
                    bridge_reachable: None,
                    bridge_error: None,
                };
            }
        },
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!("Failed to read CEF response body: {error}")),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let tabs_found = tabs.len();

    eprintln!(
        "[CEF_INJECT] Found {} tab(s) on port {}, filtering by '{}'",
        tabs_found, debug_port, target_url
    );

    let matched_tabs: Vec<&CefTab> = tabs
        .iter()
        .filter(|tab| {
            tab.url
                .as_deref()
                .is_some_and(|url| url.contains(target_url))
                || tab
                    .title
                    .as_deref()
                    .is_some_and(|title| title.contains(target_url))
        })
        .collect();

    let tabs_matched = matched_tabs.len();

    if matched_tabs.is_empty() {
        return InjectResult {
            success: false,
            tab_title: None,
            tab_url: None,
            error: Some(format!(
                "No tab matches '{target_url}'. Found {tabs_found} tabs total."
            )),
            tabs_found,
            tabs_matched,
            injected_tab_urls: Vec::new(),
            injected_target_ids: Vec::new(),
            tabs_already_injected: 0,
            debug_port,
            bridge_reachable: None,
            bridge_error: None,
        };
    }

    let mut injected_urls = Vec::new();
    let mut injected_target_ids = Vec::new();
    let mut injected_ws_urls: Vec<Option<String>> = Vec::new();
    let mut injected_script_ids: Vec<Option<String>> = Vec::new();
    let mut tabs_already_injected = 0;
    let mut first_error = None;
    let mut first_tab_title = None;
    let mut first_tab_url = None;
    let mut any_bridge_ok = false;
    let mut first_bridge_error: Option<String> = None;

    for tab in matched_tabs {
        let tab_url = tab.url.clone().unwrap_or_default();

        if first_tab_title.is_none() {
            first_tab_title = tab.title.clone();
            first_tab_url = tab.url.clone();
        }

        let tab_target_id = tab.id.clone().unwrap_or_default();
        if skip_target_ids.contains(&tab_target_id) && !tab_target_id.is_empty() {
            if let Some(ws_url) = &tab.web_socket_debugger_url {
                match verify_lifecycle(ws_url) {
                    Ok(true) => {
                        tabs_already_injected += 1;
                        eprintln!(
                            "[CEF_INJECT] Existing lifecycle verified: {}",
                            truncate_url(&tab_target_id, 40)
                        );
                        continue;
                    }
                    Ok(false) => {
                        eprintln!(
                            "[CEF_INJECT] Lifecycle missing; reinjecting target: {}",
                            truncate_url(&tab_target_id, 40)
                        );
                    }
                    Err(error) => {
                        eprintln!(
                            "[CEF_INJECT] Lifecycle verification failed for tab {}: {error}",
                            truncate_url(&tab_url, 80)
                        );
                        tabs_already_injected += 1;
                        continue;
                    }
                }
            } else {
                tabs_already_injected += 1;
                continue;
            }
        }

        eprintln!(
            "[CEF_INJECT] Injecting into tab: url=\"{}\" title=\"{}\" targetId=\"{}\"",
            truncate_url(&tab_url, 80),
            tab.title.as_deref().unwrap_or("(no title)"),
            truncate_url(&tab_target_id, 40)
        );

        let (success, error, bridge_ok, bridge_error) = inject_into_tab(tab, js_code);

        if let Some(ok) = bridge_ok {
            if ok {
                any_bridge_ok = true;
            }
        }
        if first_bridge_error.is_none() {
            first_bridge_error = bridge_error;
        }

        if success {
            injected_urls.push(tab_url.clone());
            if !tab_target_id.is_empty() {
                injected_target_ids.push(tab_target_id.clone());
            }
            injected_ws_urls.push(tab.web_socket_debugger_url.clone());
            injected_script_ids.push(None);
        } else {
            let error = error.unwrap_or_else(|| "Unknown injection error".to_string());

            eprintln!(
                "[CEF_INJECT] Injection FAILED: {} error=\"{}\"",
                truncate_url(&tab_url, 80),
                error
            );

            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }

    let success = !injected_urls.is_empty();

    InjectResult {
        success,
        tab_title: first_tab_title,
        tab_url: first_tab_url,
        error: if success { None } else { first_error },
        tabs_found,
        tabs_matched,
        injected_tab_urls: injected_urls,
        injected_target_ids,
        tabs_already_injected,
        debug_port,
        bridge_reachable: if success { Some(any_bridge_ok) } else { None },
        bridge_error: if success { first_bridge_error } else { None },
    }
}

#[tauri::command]
pub fn inject_to_steam_tab(
    target_url: String,
    js_code: String,
    skip_tab_urls: Option<Vec<String>>,
) -> InjectResult {
    inject_code_into_tabs(&target_url, &js_code, &skip_tab_urls.unwrap_or_default())
}

#[tauri::command]
pub fn inject_plugin_by_id(extension_id: String) -> InjectResult {
    let plugin = {
        let cache = super::plugins::get_plugins_cache();
        cache.get(&extension_id).map(|entry| entry.clone())
    };

    let plugin = match plugin {
        Some(plugin) => plugin,
        None => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!("Plugin '{extension_id}' not found in cache")),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    if plugin.cef_injection != Some(true) {
        return InjectResult {
            success: false,
            tab_title: None,
            tab_url: None,
            error: Some(format!(
                "Plugin '{extension_id}' does not have CEF injection enabled"
            )),
            tabs_found: 0,
            tabs_matched: 0,
            injected_tab_urls: Vec::new(),
            injected_target_ids: Vec::new(),
            tabs_already_injected: 0,
            debug_port: 0,
            bridge_reachable: None,
            bridge_error: None,
        };
    }

    let inject_script = match &plugin.inject_script {
        Some(script) => script.clone(),
        None => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Plugin '{extension_id}' has no injectScript defined"
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let target_url = plugin
        .target_url
        .clone()
        .unwrap_or_else(|| "store.steampowered.com".to_string());

    let script_path = match crate::config::resolve_inject_script(&extension_id, &inject_script) {
        Some(path) => path,
        None => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Cannot resolve inject script '{}' for plugin '{}'",
                    inject_script, extension_id
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    let js_code = match std::fs::read_to_string(&script_path) {
        Ok(code) => code,
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Failed to read inject script {}: {error}",
                    script_path.display()
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                injected_target_ids: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
                bridge_reachable: None,
                bridge_error: None,
            };
        }
    };

    eprintln!(
        "[CEF_INJECT] Manual injection: plugin='{}', target='{}', script={}",
        extension_id,
        target_url,
        script_path.display()
    );

    let skip = get_injected_target_ids(&extension_id);

    let result = inject_code_into_tabs(&target_url, &js_code, &skip);

    if result.success && !result.injected_tab_urls.is_empty() {
        track_injection(
            &extension_id,
            &target_url,
            &result.injected_target_ids,
            &result.injected_tab_urls,
            &std::iter::repeat(None)
                .take(result.injected_target_ids.len())
                .collect::<Vec<_>>(),
            &std::iter::repeat(None)
                .take(result.injected_target_ids.len())
                .collect::<Vec<_>>(),
        );
    }

    result
}

#[tauri::command]
pub fn get_injection_status() -> Value {
    let map = get_injected();
    let mut status = serde_json::Map::new();

    for entry in map.iter() {
        let targets: Vec<Value> = entry
            .value()
            .injected_targets
            .iter()
            .map(|t| {
                json!({
                    "targetId": t.target_id,
                    "lastUrl": t.last_url,
                    "wsUrl": t.ws_url,
                    "version": t.version,
                    "scriptIdentifier": t.script_identifier,
                    "lastVerified": t.last_verified.map(|i| i.elapsed().as_secs()),
                })
            })
            .collect();

        status.insert(
            entry.key().clone(),
            json!({
                "targetUrl": entry.value().target_url,
                "injectedTargets": targets,
                "injectedTabs": entry.value().injected_targets.len(),
            }),
        );
    }

    Value::Object(status)
}

// ---------------------------------------------------------------------------
// Target monitor — polls /json for new CDP targets matching the extension
// ---------------------------------------------------------------------------

static CEF_MONITOR_RUNNING: AtomicBool = AtomicBool::new(false);

pub fn start_target_monitor(extension_id: String, target_url: String) {
    if CEF_MONITOR_RUNNING.swap(true, Ordering::SeqCst) {
        eprintln!("[CEF_MONITOR] Already running, skipping start");
        return;
    }

    eprintln!("[CEF_MONITOR] Started for extension: {}", extension_id);

    thread::spawn(move || {
        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[CEF_MONITOR] Failed to create HTTP client: {e}");
                CEF_MONITOR_RUNNING.store(false, Ordering::SeqCst);
                return;
            }
        };

        while CEF_MONITOR_RUNNING.load(Ordering::SeqCst) {
            let debug_port = detect_cef_debug_port();
            if let Some(port) = debug_port {
                let endpoint = format!("http://127.0.0.1:{port}/json");
                if let Ok(resp) = client.get(&endpoint).send() {
                    if let Ok(raw) = resp.text() {
                        if let Ok(tabs) = serde_json::from_str::<Vec<CefTab>>(&raw) {
                            let current_target_ids: Vec<String> = tabs
                                .iter()
                                .filter(|t| {
                                    t.url.as_deref().is_some_and(|u| u.contains(&target_url))
                                })
                                .filter_map(|t| t.id.clone())
                                .collect();

                            let injected_ids = get_injected_target_ids(&extension_id);

                            for tab in &tabs {
                                let tab_id = match &tab.id {
                                    Some(id) => id.clone(),
                                    None => continue,
                                };
                                let tab_url = tab.url.as_deref().unwrap_or("");
                                if !tab_url.contains(&target_url) {
                                    continue;
                                }

                                if injected_ids.contains(&tab_id) {
                                    if let Some(ws_url) = &tab.web_socket_debugger_url {
                                        match verify_lifecycle(ws_url) {
                                            Ok(true) => {
                                                eprintln!(
                                                    "[CEF_MONITOR] Existing lifecycle verified: {}",
                                                    truncate_url(&tab_id, 40)
                                                );
                                            }
                                            Ok(false) => {
                                                eprintln!(
                                                    "[CEF_MONITOR] Lifecycle missing; reinjecting target: {}",
                                                    truncate_url(&tab_id, 40)
                                                );
                                                if let Some(ws) = &tab.web_socket_debugger_url {
                                                    let inject_script =
                                                        super::plugins::get_plugins_cache()
                                                            .get(&extension_id)
                                                            .and_then(|p| p.inject_script.clone())
                                                            .unwrap_or_else(|| {
                                                                "inject.js".to_string()
                                                            });
                                                    if let Some(script_path) =
                                                        crate::config::resolve_inject_script(
                                                            &extension_id,
                                                            &inject_script,
                                                        )
                                                    {
                                                        if let Ok(js_code) =
                                                            std::fs::read_to_string(&script_path)
                                                        {
                                                            let report =
                                                                connect_once(ws, &js_code, &tab_id);
                                                            if report.lifecycle_ok {
                                                                eprintln!(
                                                                    "[CEF_MONITOR] Reinjection successful for target: {}",
                                                                    truncate_url(&tab_id, 40)
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!(
                                                    "[CEF_MONITOR] Lifecycle check failed for {}: {e}",
                                                    truncate_url(&tab_id, 40)
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    eprintln!(
                                        "[CEF_MONITOR] New Store target detected: {}",
                                        truncate_url(&tab_id, 40)
                                    );
                                }
                            }

                            let stale_ids: Vec<String> = injected_ids
                                .iter()
                                .filter(|id| !current_target_ids.contains(id))
                                .cloned()
                                .collect();
                            for stale_id in &stale_ids {
                                eprintln!(
                                    "[CEF_MONITOR] Target removed: {}",
                                    truncate_url(stale_id, 40)
                                );
                            }
                        }
                    }
                }
            }

            thread::sleep(Duration::from_millis(TARGET_MONITOR_INTERVAL_MS));
        }

        eprintln!("[CEF_MONITOR] Stopped for extension: {}", extension_id);
    });
}

pub fn stop_target_monitor() {
    CEF_MONITOR_RUNNING.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_monitor_starts_and_stops() {
        let was_running = CEF_MONITOR_RUNNING.load(Ordering::SeqCst);
        if !was_running {
            CEF_MONITOR_RUNNING.store(true, Ordering::SeqCst);
            stop_target_monitor();
            assert!(!CEF_MONITOR_RUNNING.load(Ordering::SeqCst));
        }
    }

    #[test]
    fn track_injection_stores_target_data() {
        let ext_id = "test-track-target";
        let target_url = "store.steampowered.com";
        let target_ids = vec!["target-1".to_string()];
        let urls = vec!["https://store.steampowered.com/app/730".to_string()];
        let ws_urls = vec![Some("ws://127.0.0.1:8080/devtools/page/abc".to_string())];
        let script_ids = vec![Some("script-id-1".to_string())];

        track_injection(
            ext_id,
            target_url,
            &target_ids,
            &urls,
            &ws_urls,
            &script_ids,
        );

        {
            let injected = get_injected();
            assert!(injected.contains_key(ext_id));
            let entry = injected.get(ext_id).unwrap();
            assert_eq!(entry.injected_targets.len(), 1);
            assert_eq!(entry.injected_targets[0].target_id, "target-1");
            assert_eq!(
                entry.injected_targets[0].ws_url,
                Some("ws://127.0.0.1:8080/devtools/page/abc".to_string())
            );
            assert_eq!(
                entry.injected_targets[0].script_identifier,
                Some("script-id-1".to_string())
            );
            assert_eq!(entry.injected_targets[0].version, LUMA_INJECT_VERSION);
        }
        clear_injection(ext_id);
    }

    #[test]
    fn clear_injection_removes_entry() {
        let ext_id = "test-clear-injection";
        track_injection(
            ext_id,
            "store.steampowered.com",
            &["t1".to_string()],
            &["url1".to_string()],
            &[None],
            &[None],
        );
        assert!(get_injected().contains_key(ext_id));
        clear_injection(ext_id);
        assert!(!get_injected().contains_key(ext_id));
    }

    #[test]
    fn get_injected_target_ids_returns_ids() {
        let ext_id = "test-get-ids";
        track_injection(
            ext_id,
            "store.steampowered.com",
            &["a".to_string(), "b".to_string()],
            &["u1".to_string(), "u2".to_string()],
            &[None, None],
            &[None, None],
        );
        let ids = get_injected_target_ids(ext_id);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"a".to_string()));
        assert!(ids.contains(&"b".to_string()));
        clear_injection(ext_id);
    }

    #[test]
    fn get_injected_target_ids_returns_empty_for_unknown() {
        assert!(get_injected_target_ids("nonexistent").is_empty());
    }

    #[test]
    fn inject_result_has_bridge_fields() {
        let result = InjectResult {
            success: false,
            tab_title: None,
            tab_url: None,
            error: None,
            tabs_found: 0,
            tabs_matched: 0,
            injected_tab_urls: vec![],
            injected_target_ids: vec![],
            tabs_already_injected: 0,
            debug_port: 0,
            bridge_reachable: Some(true),
            bridge_error: None,
        };
        assert_eq!(result.bridge_reachable, Some(true));
        assert!(result.bridge_error.is_none());
    }

    #[test]
    fn inject_result_bridge_failure() {
        let result = InjectResult {
            success: true,
            tab_title: None,
            tab_url: None,
            error: None,
            tabs_found: 1,
            tabs_matched: 1,
            injected_tab_urls: vec!["url".into()],
            injected_target_ids: vec!["tid".into()],
            tabs_already_injected: 0,
            debug_port: 8080,
            bridge_reachable: Some(false),
            bridge_error: Some("fetch failed".into()),
        };
        assert_eq!(result.bridge_reachable, Some(false));
        assert_eq!(result.bridge_error.as_deref(), Some("fetch failed"));
    }

    #[test]
    fn target_url_matching_works() {
        assert!("https://store.steampowered.com/app/730".contains("store.steampowered.com"));
        assert!("https://store.steampowered.com/app/570/store".contains("store.steampowered.com"));
        assert!(!"https://steamcommunity.com/profiles/123".contains("store.steampowered.com"));
    }

    #[test]
    fn version_constant_matches_inject_js() {
        assert_eq!(LUMA_INJECT_VERSION, "2.5.0-download-flow");
    }

    #[test]
    fn inject_result_serializes_bridge_fields() {
        let result = InjectResult {
            success: true,
            tab_title: None,
            tab_url: None,
            error: None,
            tabs_found: 1,
            tabs_matched: 1,
            injected_tab_urls: vec![],
            injected_target_ids: vec![],
            tabs_already_injected: 0,
            debug_port: 8080,
            bridge_reachable: Some(true),
            bridge_error: None,
        };
        let v = serde_json::to_value(&result).unwrap();
        assert_eq!(v["bridgeReachable"], true);
        assert!(v["bridgeError"].is_null());
    }

    #[test]
    fn monitor_stop_sets_flag() {
        CEF_MONITOR_RUNNING.store(true, Ordering::SeqCst);
        stop_target_monitor();
        assert!(!CEF_MONITOR_RUNNING.load(Ordering::SeqCst));
    }

    #[test]
    fn injection_status_includes_version_and_ws_url() {
        let ext_id = "test-status-fields";
        track_injection(
            ext_id,
            "store.steampowered.com",
            &["t1".to_string()],
            &["u1".to_string()],
            &[Some("ws://localhost:8080/devtools/page/abc".to_string())],
            &[Some("id-123".to_string())],
        );

        let status = get_injection_status();
        let entry_val = status.get(ext_id).cloned().unwrap();
        assert!(entry_val["injectedTargets"].is_array());
        let targets = entry_val["injectedTargets"].as_array().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0]["version"], LUMA_INJECT_VERSION);
        assert_eq!(targets[0]["wsUrl"], "ws://localhost:8080/devtools/page/abc");
        assert_eq!(targets[0]["scriptIdentifier"], "id-123");

        clear_injection(ext_id);
    }

    #[test]
    fn connect_report_structure() {
        let report = ConnectReport {
            lifecycle_ok: true,
            bridge_ok: false,
            lifecycle_error: None,
            bridge_error: Some("bridge down".to_string()),
        };
        assert!(report.lifecycle_ok);
        assert!(!report.bridge_ok);
        assert!(report.lifecycle_error.is_none());
        assert_eq!(report.bridge_error.as_deref(), Some("bridge down"));
    }

    #[test]
    fn multiple_targets_tracked_independently() {
        let ext_id = "test-multi-targets";
        track_injection(
            ext_id,
            "store.steampowered.com",
            &["t1".to_string(), "t2".to_string(), "t3".to_string()],
            &["u1".to_string(), "u2".to_string(), "u3".to_string()],
            &[Some("ws://1".to_string()), Some("ws://2".to_string()), None],
            &[Some("sid-1".to_string()), None, Some("sid-3".to_string())],
        );

        let ids = get_injected_target_ids(ext_id);
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"t1".to_string()));
        assert!(ids.contains(&"t2".to_string()));
        assert!(ids.contains(&"t3".to_string()));

        {
            let injected = get_injected();
            let entry = injected.get(ext_id).unwrap();
            assert_eq!(entry.injected_targets[0].ws_url, Some("ws://1".to_string()));
            assert_eq!(entry.injected_targets[1].ws_url, Some("ws://2".to_string()));
            assert_eq!(entry.injected_targets[2].ws_url, None);
            assert_eq!(
                entry.injected_targets[0].script_identifier,
                Some("sid-1".to_string())
            );
            assert_eq!(entry.injected_targets[1].script_identifier, None);
        }
        clear_injection(ext_id);
    }
}
