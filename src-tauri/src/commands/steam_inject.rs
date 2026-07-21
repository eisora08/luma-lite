use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::TcpStream;
use std::sync::OnceLock;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

const CEF_DEBUG_PORTS: &[u16] = &[8080, 8081, 8082, 8083, 9222];
const HTTP_TIMEOUT_SECS: u64 = 3;
const WS_RETRY_ATTEMPTS: u32 = 2;

const CDP_ID_PAGE_ENABLE: u64 = 1;
const CDP_ID_RUNTIME_ENABLE: u64 = 2;
const CDP_ID_BYPASS_CSP: u64 = 3;
const CDP_ID_INJECT_SCRIPT: u64 = 4;
const CDP_ID_NETWORK_DIAGNOSTIC: u64 = 5;

#[derive(Debug, Deserialize)]
struct CefTab {
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
    pub tabs_already_injected: usize,
    pub debug_port: u16,
}

struct InjectedExtension {
    target_url: String,
    injected_urls: Vec<String>,
}

static INJECTED: OnceLock<DashMap<String, InjectedExtension>> = OnceLock::new();

fn get_injected() -> &'static DashMap<String, InjectedExtension> {
    INJECTED.get_or_init(DashMap::new)
}

pub fn track_injection(extension_id: &str, target_url: &str, urls: &[String]) {
    get_injected().insert(
        extension_id.to_string(),
        InjectedExtension {
            target_url: target_url.to_string(),
            injected_urls: urls.to_vec(),
        },
    );
}

pub fn clear_injection(extension_id: &str) {
    get_injected().remove(extension_id);
}

pub fn get_injected_urls(extension_id: &str) -> Vec<String> {
    get_injected()
        .get(extension_id)
        .map(|entry| entry.injected_urls.clone())
        .unwrap_or_default()
}

pub(crate) fn detect_cef_debug_port() -> Option<u16> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            eprintln!(
                "[CEF_INJECT] Failed to create CEF detection client: {error}"
            );
            return None;
        }
    };

    for &port in CEF_DEBUG_PORTS {
        let url = format!("http://127.0.0.1:{port}/json");

        match client.get(&url).send() {
            Ok(response) if response.status().is_success() => {
                eprintln!(
                    "[CEF_INJECT] Detected CEF debugger on port {port}"
                );
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

                if let Some(exception) =
                    parsed.pointer("/result/exceptionDetails")
                {
                    return Err(format_exception_details(
                        command_name,
                        exception,
                    ));
                }

                return Ok(parsed);
            }

            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .map_err(|error| {
                        format!(
                            "{command_name} failed to answer WebSocket ping: {error}"
                        )
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

fn run_network_diagnostic(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<(), String> {
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

    eprintln!(
        "[CDP_DIAG] Runtime.evaluate response: {}",
        response
    );

    let value = extract_returned_value(&response).ok_or_else(|| {
        format!(
            "Network diagnostic returned no value: {}",
            response
        )
    })?;

    let reached = value
        .get("reached")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if !reached {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Error");

        let message = value
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Unknown fetch error");

        let href = value
            .get("href")
            .and_then(Value::as_str)
            .unwrap_or("");

        let origin = value
            .get("origin")
            .and_then(Value::as_str)
            .unwrap_or("");

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

    let ok = value
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let response_url = value
        .get("responseUrl")
        .and_then(Value::as_str)
        .unwrap_or("");

    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("");

    eprintln!(
        "[CDP_DIAG] Provider fetch reached bridge: status={status}, ok={ok}, response_url={response_url}, body={body}"
    );

    if !ok {
        return Err(format!(
            "Provider endpoint returned HTTP {status}: {body}"
        ));
    }

    Ok(())
}

fn connect_once(ws_url: &str, js_code: &str) -> Result<(), String> {
    let (mut socket, _) =
        connect(ws_url).map_err(|error| {
            format!("WebSocket connect failed: {error}")
        })?;

    let page_enable = json!({
        "id": CDP_ID_PAGE_ENABLE,
        "method": "Page.enable"
    });

    let page_enable_response = send_cdp_command(
        &mut socket,
        &page_enable,
        CDP_ID_PAGE_ENABLE,
        "Page.enable",
    )?;

    eprintln!(
        "[CEF_INJECT] Page.enable response: {}",
        page_enable_response
    );

    let runtime_enable = json!({
        "id": CDP_ID_RUNTIME_ENABLE,
        "method": "Runtime.enable"
    });

    let runtime_enable_response = send_cdp_command(
        &mut socket,
        &runtime_enable,
        CDP_ID_RUNTIME_ENABLE,
        "Runtime.enable",
    )?;

    eprintln!(
        "[CEF_INJECT] Runtime.enable response: {}",
        runtime_enable_response
    );

    let bypass_csp = json!({
        "id": CDP_ID_BYPASS_CSP,
        "method": "Page.setBypassCSP",
        "params": {
            "enabled": true
        }
    });

    let bypass_response = send_cdp_command(
        &mut socket,
        &bypass_csp,
        CDP_ID_BYPASS_CSP,
        "Page.setBypassCSP",
    )?;

    eprintln!(
        "[CEF_INJECT] Page.setBypassCSP response: {}",
        bypass_response
    );

    let inject_command = json!({
        "id": CDP_ID_INJECT_SCRIPT,
        "method": "Runtime.evaluate",
        "params": {
            "expression": js_code,
            "awaitPromise": true,
            "returnByValue": true
        }
    });

    let inject_response = send_cdp_command(
        &mut socket,
        &inject_command,
        CDP_ID_INJECT_SCRIPT,
        "Runtime.evaluate injection",
    )?;

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
                .unwrap_or("JavaScript evaluation returned an error");

            let _ = socket.close(None);

            return Err(format!(
                "Runtime.evaluate returned an error result: {description}"
            ));
        }
    }

    match run_network_diagnostic(&mut socket) {
        Ok(()) => {
            eprintln!(
                "[CDP_DIAG] Diagnostic provider fetch succeeded"
            );
        }
        Err(error) => {
            eprintln!(
                "[CDP_DIAG] Diagnostic provider fetch failed: {error}"
            );
        }
    }

    let _ = socket.close(None);

    Ok(())
}

fn inject_into_tab(tab: &CefTab, js_code: &str) -> (bool, Option<String>) {
    let ws_url = match &tab.web_socket_debugger_url {
        Some(url) => url.clone(),
        None => {
            return (
                false,
                Some(
                    "No webSocketDebuggerUrl available for this tab"
                        .to_string(),
                ),
            );
        }
    };

    let mut last_error = None;

    for attempt in 0..=WS_RETRY_ATTEMPTS {
        if attempt > 0 {
            eprintln!(
                "[CEF_INJECT] Retry attempt {}/{} for tab WebSocket: {}",
                attempt,
                WS_RETRY_ATTEMPTS,
                truncate_url(&ws_url, 60)
            );

            std::thread::sleep(std::time::Duration::from_millis(
                200 * u64::from(attempt),
            ));
        }

        match connect_once(&ws_url, js_code) {
            Ok(()) => return (true, None),
            Err(error) => {
                last_error = Some(error);
            }
        }
    }

    (false, last_error)
}

fn truncate_url(url: &str, max_len: usize) -> String {
    if url.len() <= max_len {
        url.to_string()
    } else {
        format!("{}...", &url[..max_len - 3])
    }
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
                tabs_already_injected: 0,
                debug_port: 0,
            };
        }
    };

    let debug_endpoint =
        format!("http://127.0.0.1:{debug_port}/json");

    let skip_set: HashSet<String> =
        skip_tab_urls.iter().cloned().collect();

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(
            HTTP_TIMEOUT_SECS,
        ))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Failed to build HTTP client: {error}"
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                tabs_already_injected: 0,
                debug_port,
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
                tabs_already_injected: 0,
                debug_port,
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
                    error: Some(format!(
                        "Failed to parse CEF tab list: {error}"
                    )),
                    tabs_found: 0,
                    tabs_matched: 0,
                    injected_tab_urls: Vec::new(),
                    tabs_already_injected: 0,
                    debug_port,
                };
            }
        },
        Err(error) => {
            return InjectResult {
                success: false,
                tab_title: None,
                tab_url: None,
                error: Some(format!(
                    "Failed to read CEF response body: {error}"
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                tabs_already_injected: 0,
                debug_port,
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
            tabs_already_injected: 0,
            debug_port,
        };
    }

    let mut injected_urls = Vec::new();
    let mut tabs_already_injected = 0;
    let mut first_error = None;
    let mut first_tab_title = None;
    let mut first_tab_url = None;

    for tab in matched_tabs {
        let tab_url = tab.url.clone().unwrap_or_default();

        if first_tab_title.is_none() {
            first_tab_title = tab.title.clone();
            first_tab_url = tab.url.clone();
        }

        if skip_set.contains(&tab_url) {
            tabs_already_injected += 1;

            eprintln!(
                "[CEF_INJECT] Skipping already-injected tab: {}",
                truncate_url(&tab_url, 80)
            );

            continue;
        }

        eprintln!(
            "[CEF_INJECT] Injecting into tab: url=\"{}\" title=\"{}\"",
            truncate_url(&tab_url, 80),
            tab.title.as_deref().unwrap_or("(no title)")
        );

        let (success, error) = inject_into_tab(tab, js_code);

        if success {
            eprintln!(
                "[CEF_INJECT] Injection SUCCESS: {}",
                truncate_url(&tab_url, 80)
            );

            injected_urls.push(tab_url);
        } else {
            let error =
                error.unwrap_or_else(|| "Unknown injection error".to_string());

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
        tabs_already_injected,
        debug_port,
    }
}

#[tauri::command]
pub fn inject_to_steam_tab(
    target_url: String,
    js_code: String,
    skip_tab_urls: Option<Vec<String>>,
) -> InjectResult {
    inject_code_into_tabs(
        &target_url,
        &js_code,
        &skip_tab_urls.unwrap_or_default(),
    )
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
                error: Some(format!(
                    "Plugin '{extension_id}' not found in cache"
                )),
                tabs_found: 0,
                tabs_matched: 0,
                injected_tab_urls: Vec::new(),
                tabs_already_injected: 0,
                debug_port: 0,
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
            tabs_already_injected: 0,
            debug_port: 0,
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
                tabs_already_injected: 0,
                debug_port: 0,
            };
        }
    };

    let target_url = plugin
        .target_url
        .clone()
        .unwrap_or_else(|| "store.steampowered.com".to_string());

    let script_path = match crate::config::resolve_inject_script(
        &extension_id,
        &inject_script,
    ) {
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
                tabs_already_injected: 0,
                debug_port: 0,
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
                tabs_already_injected: 0,
                debug_port: 0,
            };
        }
    };

    eprintln!(
        "[CEF_INJECT] Manual injection: plugin='{}', target='{}', script={}",
        extension_id,
        target_url,
        script_path.display()
    );

    let skip = get_injected_urls(&extension_id);

    let result =
        inject_code_into_tabs(&target_url, &js_code, &skip);

    if result.success && !result.injected_tab_urls.is_empty() {
        track_injection(
            &extension_id,
            &target_url,
            &result.injected_tab_urls,
        );
    }

    result
}

#[tauri::command]
pub fn get_injection_status() -> Value {
    let map = get_injected();
    let mut status = serde_json::Map::new();

    for entry in map.iter() {
        status.insert(
            entry.key().clone(),
            json!({
                "targetUrl": entry.value().target_url,
                "injectedTabs": entry.value().injected_urls.len(),
                "injectedUrls": entry.value().injected_urls
            }),
        );
    }

    Value::Object(status)
}
