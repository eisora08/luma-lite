use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::Emitter;

use super::hubcap;

const BRIDGE_PORT: u16 = 21775;
const BRIDGE_ADDR: &str = "127.0.0.1";

static BRIDGE_RUNNING: AtomicBool = AtomicBool::new(false);
static BRIDGE_READY_FLAG: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Download job tracking
// ---------------------------------------------------------------------------

struct DownloadJob {
    request_id: String,
    app_id: String,
    provider_id: String,
    status: String,
    progress: Option<u32>,
    message: String,
    error_code: Option<String>,
    output_type: Option<String>,
    started_at: Instant,
    completed_at: Option<Instant>,
}

fn job_store() -> &'static Mutex<HashMap<String, DownloadJob>> {
    static JOBS: OnceLock<Mutex<HashMap<String, DownloadJob>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn generate_request_id() -> String {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let pid = std::process::id();
    format!("req-{pid}-{ts}")
}

fn bridge_sync() -> &'static (Mutex<bool>, Condvar) {
    static SYNC: OnceLock<(Mutex<bool>, Condvar)> = OnceLock::new();
    SYNC.get_or_init(|| (Mutex::new(false), Condvar::new()))
}

pub fn wait_until_bridge_ready(timeout_ms: u64) -> bool {
    if BRIDGE_READY_FLAG.load(Ordering::SeqCst) {
        return true;
    }
    let (lock, cv) = bridge_sync();
    let ready = lock.lock().unwrap();
    if *ready {
        return true;
    }
    let _ = cv.wait_timeout(ready, std::time::Duration::from_millis(timeout_ms));
    BRIDGE_READY_FLAG.load(Ordering::SeqCst)
}

const MAX_PACKAGE_ID_LENGTH: usize = 12;

pub fn start_steam_bridge(app_handle: tauri::AppHandle) {
    if BRIDGE_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    let pid = std::process::id();
    let exe_path = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "(unknown)".into());

    eprintln!("[BRIDGE_ID] Starting local bridge");
    eprintln!("[BRIDGE_ID] PID: {pid}");
    eprintln!("[BRIDGE_ID] Executable: {exe_path}");
    eprintln!("[BRIDGE_ID] Binding to: {BRIDGE_ADDR}:{BRIDGE_PORT}");
    eprintln!("[BRIDGE_ID] Bridge implementation: steam_bridge::start_steam_bridge");
    eprintln!("[BRIDGE_ID] Route table version: forensic-1");

    thread::spawn(move || {
        let addr = format!("{BRIDGE_ADDR}:{BRIDGE_PORT}");

        let listener = {
            let mut delay_ms = 200u64;
            loop {
                match TcpListener::bind(&addr) {
                    Ok(l) => break l,
                    Err(e) => {
                        let err = e.to_string();
                        if delay_ms > 10_000 {
                            eprintln!("[STEAM_BRIDGE] Failed to bind {addr} after retries: {err}");
                            BRIDGE_RUNNING.store(false, Ordering::SeqCst);
                            return;
                        }
                        eprintln!("[STEAM_BRIDGE] Bind failed (retrying in {delay_ms}ms): {err}");
                        thread::sleep(std::time::Duration::from_millis(delay_ms));
                        delay_ms = delay_ms.saturating_mul(2);
                    }
                }
            }
        };

        let _ = listener.set_nonblocking(true);

        BRIDGE_READY_FLAG.store(true, Ordering::SeqCst);
        {
            let (lock, cv) = bridge_sync();
            let mut ready = lock.lock().unwrap();
            *ready = true;
            cv.notify_all();
        }

        eprintln!("[STEAM_BRIDGE] Ready on http://{addr}");
        eprintln!("[STEAM_BRIDGE] Listening on http://{addr}");
        eprintln!("  Endpoints:");
        eprintln!("    GET  /health");
        eprintln!("    GET  /api/providers");
        eprintln!("    GET  /api/sources/{{appId}}");
        eprintln!("    GET  /api/local-status/{{appId}}");
        eprintln!("    GET  /api/settings");
        eprintln!("    POST /api/download");
        eprintln!("    POST /api/settings");
        eprintln!("    GET|POST /api/download-package/{{appId}}");
        eprintln!("    GET  /api/download-status/{{requestId}}");
        eprintln!("    POST /api/open-library/{{appId}}");

        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let _ = stream.set_nonblocking(false);
                    let handle = app_handle.clone();
                    thread::spawn(move || {
                        handle_request(&mut stream, &handle);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::ConnectionAborted
                        && e.kind() != std::io::ErrorKind::ConnectionReset
                        && e.kind() != std::io::ErrorKind::Interrupted
                    {
                        eprintln!("[STEAM_BRIDGE] Accept error: {e}");
                    }
                }
            }
        }

        BRIDGE_RUNNING.store(false, Ordering::SeqCst);
        eprintln!("[STEAM_BRIDGE] Stopped");
    });
}

// ---------------------------------------------------------------------------
// Package ID validation
// ---------------------------------------------------------------------------

fn is_valid_package_id(id: &str) -> bool {
    if id.is_empty() || id.len() > MAX_PACKAGE_ID_LENGTH {
        return false;
    }
    id.chars().all(|c| c.is_ascii_digit()) && id != "0"
}

// ---------------------------------------------------------------------------
// Route actions (decoupled from AppHandle for testability)
// ---------------------------------------------------------------------------

enum RouteAction {
    None,
    EmitDownloadPackage(String),
    EmitDownloadProvider {
        app_id: String,
        request_id: String,
        provider_id: String,
    },
    OpenLibrary {
        uri: String,
    },
}

// ---------------------------------------------------------------------------
// Request handling
// ---------------------------------------------------------------------------

fn handle_request(stream: &mut TcpStream, app_handle: &tauri::AppHandle) {
    let mut buffer = [0; 4096];
    let bytes_read = match stream.read(&mut buffer) {
        Ok(0) => return,
        Ok(n) => n,
        Err(e) => {
            eprintln!("[STEAM_BRIDGE] Read error: {e}");
            return;
        }
    };

    let request = String::from_utf8_lossy(&buffer[..bytes_read]);

    let req_method = request.split_whitespace().next().unwrap_or("?");
    let req_path = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?");
    let req_origin = request
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("origin:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|v| v.trim())
        .unwrap_or("(none)");
    let req_host = request
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("host:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|v| v.trim())
        .unwrap_or("(none)");
    let req_ct = request
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-type:"))
        .and_then(|l| l.splitn(2, ':').nth(1))
        .map(|v| v.trim())
        .unwrap_or("(none)");
    let pid = std::process::id();
    eprintln!("[BRIDGE_REQUEST] PID: {pid}");
    eprintln!("[BRIDGE_REQUEST] METHOD: {req_method}");
    eprintln!("[BRIDGE_REQUEST] PATH: {req_path}");
    eprintln!("[BRIDGE_REQUEST] ORIGIN: {req_origin}");
    eprintln!("[BRIDGE_REQUEST] HOST: {req_host}");
    eprintln!("[BRIDGE_REQUEST] CONTENT_TYPE: {req_ct}");

    let (status_line, cors_headers, body, action) = route_request(&request);

    match action {
        RouteAction::EmitDownloadPackage(app_id) => {
            let _ = app_handle.emit(
                "steam-bridge://download-package",
                serde_json::json!({ "appId": app_id }),
            );
        }
        RouteAction::EmitDownloadProvider {
            app_id,
            request_id,
            provider_id,
        } => {
            let handle = app_handle.clone();
            let rid = request_id.clone();
            let pid = provider_id.clone();
            thread::spawn(move || {
                // Phase: validating
                {
                    let mut jobs = job_store().lock().unwrap();
                    if let Some(job) = jobs.get_mut(&rid) {
                        job.status = "validating".to_string();
                        job.message = "Validating package.".to_string();
                    }
                }
                let _ = handle.emit(
                    "steam-bridge://download-progress",
                    serde_json::json!({ "appId": app_id, "requestId": rid, "status": "validating" }),
                );

                // Phase: checking_availability
                {
                    let mut jobs = job_store().lock().unwrap();
                    if let Some(job) = jobs.get_mut(&rid) {
                        job.status = "checking_availability".to_string();
                        job.message = "Checking provider availability.".to_string();
                    }
                }
                eprintln!("[DOWNLOADS] Request progress: requestId={rid}, status=checking_availability, provider={pid}");
                let _ = handle.emit(
                    "steam-bridge://download-progress",
                    serde_json::json!({ "appId": app_id, "requestId": rid, "status": "checking_availability" }),
                );

                // Phase: downloading
                {
                    let mut jobs = job_store().lock().unwrap();
                    if let Some(job) = jobs.get_mut(&rid) {
                        job.status = "downloading".to_string();
                        job.message = "Downloading package.".to_string();
                        job.progress = Some(10);
                    }
                }
                let _ = handle.emit(
                    "steam-bridge://download-progress",
                    serde_json::json!({ "appId": app_id, "requestId": rid, "status": "downloading" }),
                );

                let steam_root = crate::config::resolve_steam_root();
                let Some(root) = steam_root else {
                    let mut jobs = job_store().lock().unwrap();
                    if let Some(job) = jobs.get_mut(&rid) {
                        job.status = "failed".to_string();
                        job.error_code = Some("STEAM_ROOT_NOT_FOUND".to_string());
                        job.message = "Steam root directory not found.".to_string();
                        job.completed_at = Some(Instant::now());
                    }
                    let _ = handle.emit(
                        "steam-bridge://download-error",
                        serde_json::json!({ "appId": app_id, "requestId": rid, "error": "Steam root not found" }),
                    );
                    return;
                };

                let job_output_type = {
                    let jobs = job_store().lock().unwrap();
                    jobs.get(&rid).and_then(|j| j.output_type.clone())
                };

                // Use multi-provider fallback with the selected provider as preferred
                match super::providers::download_with_fallback(
                    &app_id,
                    &pid,
                    &root,
                    Some(&rid),
                    job_output_type.as_deref(),
                ) {
                    Ok(result) => {
                        {
                            let mut jobs = job_store().lock().unwrap();
                            if let Some(job) = jobs.get_mut(&rid) {
                                job.status = "extracting".to_string();
                                job.message = "Extracting package contents.".to_string();
                                job.progress = Some(75);
                            }
                        }
                        let _ = handle.emit(
                            "steam-bridge://download-progress",
                            serde_json::json!({ "appId": app_id, "requestId": rid, "status": "extracting" }),
                        );

                        {
                            let mut jobs = job_store().lock().unwrap();
                            if let Some(job) = jobs.get_mut(&rid) {
                                job.status = "processing".to_string();
                                job.message = "Processing package metadata.".to_string();
                                job.progress = Some(90);
                            }
                        }
                        let _ = handle.emit(
                            "steam-bridge://download-progress",
                            serde_json::json!({ "appId": app_id, "requestId": rid, "status": "processing" }),
                        );

                        {
                            let mut jobs = job_store().lock().unwrap();
                            if let Some(job) = jobs.get_mut(&rid) {
                                job.status = "completed".to_string();
                                job.progress = Some(100);
                                job.message = "Package added successfully.".to_string();
                                job.completed_at = Some(Instant::now());
                            }
                        }
                        eprintln!("[DOWNLOADS] Request completed: requestId={rid}, appId={app_id}, provider={pid}");
                        let _ = handle.emit(
                            "steam-bridge://download-complete",
                            serde_json::json!({
                                "appId": app_id,
                                "requestId": rid,
                                "luaFiles": result.lua_files,
                                "manifestFiles": result.manifest_files,
                                "errors": result.errors,
                                "outputType": job_output_type,
                            }),
                        );
                    }
                    Err(e) => {
                        let mut jobs = job_store().lock().unwrap();
                        if let Some(job) = jobs.get_mut(&rid) {
                            job.status = "failed".to_string();
                            job.error_code = Some("DOWNLOAD_FAILED".to_string());
                            job.message = format!("Download failed: {e}");
                            job.completed_at = Some(Instant::now());
                        }
                        eprintln!("[DOWNLOADS] Request failed: requestId={rid}, error={e}");
                        let _ = handle.emit(
                            "steam-bridge://download-error",
                            serde_json::json!({ "appId": app_id, "requestId": rid, "error": e }),
                        );
                    }
                }
            });
        }
        RouteAction::OpenLibrary { uri } => {
            #[allow(deprecated)]
            {
                use tauri_plugin_shell::ShellExt;
                let _ = app_handle.shell().open(&uri, None);
            }
        }
        RouteAction::None => {}
    }

    let response = format!(
        "{status_line}\r\n\
         Content-Type: application/json\r\n\
         {cors_headers}\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        body.len(),
        body,
    );

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

const TRUSTED_ORIGINS: &[&str] = &[
    "https://store.steampowered.com",
    "https://store.steampowered.com/",
    "http://store.steampowered.com",
    "http://store.steampowered.com/",
    "null",
];

fn extract_origin(request: &str) -> Option<&str> {
    for line in request.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed
            .strip_prefix("Origin:")
            .or_else(|| trimmed.strip_prefix("origin:"))
            .or_else(|| trimmed.strip_prefix("ORIGIN:"))
        {
            return Some(val.trim());
        }
    }
    None
}

fn extract_method(request: &str) -> &str {
    let line = request.lines().next().unwrap_or("");
    let end = line.find(' ').unwrap_or(line.len());
    &line[..end]
}

fn extract_path(request: &str) -> &str {
    let line = request.lines().next().unwrap_or("");
    let start = line.find(' ').map(|i| i + 1).unwrap_or(0);
    let end = line[start..]
        .find(' ')
        .map(|i| start + i)
        .unwrap_or(line.len());
    &line[start..end]
}

fn build_cors_headers(request: &str) -> String {
    let origin = extract_origin(request);
    let method = extract_method(request);
    let path = extract_path(request);

    match &origin {
        Some(o) => eprintln!("[STEAM_BRIDGE] {} {} Origin: {}", method, path, o),
        None => eprintln!("[STEAM_BRIDGE] {} {} Origin: (none)", method, path),
    }

    let allowed_origin = match origin {
        Some(o) if TRUSTED_ORIGINS.iter().any(|t| o.eq_ignore_ascii_case(t)) => {
            if o == "null" {
                "null".to_string()
            } else {
                o.to_string()
            }
        }
        Some(o) => {
            eprintln!("[STEAM_BRIDGE] Non-trusted origin, using wildcard: {o}");
            "*".to_string()
        }
        None => "*".to_string(),
    };
    let wants_private_network = request
        .to_ascii_lowercase()
        .contains("access-control-request-private-network: true");
    let private_network_header = if wants_private_network {
        "Access-Control-Allow-Private-Network: true\r\n"
    } else {
        ""
    };
    format!(
        "Access-Control-Allow-Origin: {allowed_origin}\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type, Authorization\r\n\
         {private_network_header}\
         Access-Control-Max-Age: 86400\r\n\
         Cache-Control: no-store\r\n\
         Vary: Origin\r\n"
    )
}

fn route_request(request: &str) -> (&'static str, String, String, RouteAction) {
    if request.starts_with("OPTIONS") {
        let headers = build_cors_headers(request);
        return (
            "HTTP/1.1 204 No Content",
            headers,
            String::new(),
            RouteAction::None,
        );
    }

    if request.starts_with("GET /health") {
        let body = serde_json::json!({ "status": "ok" });
        let headers = build_cors_headers(request);
        return (
            "HTTP/1.1 200 OK",
            headers,
            body.to_string(),
            RouteAction::None,
        );
    }

    // GET /api/providers — safe public provider list (no secrets)
    if request.starts_with("GET /api/providers") {
        let config = crate::config::load_config();
        let providers: Vec<serde_json::Value> = config
            .downloads
            .providers
            .iter()
            .filter(|p| p.enabled)
            .map(|p| {
                let pub_view = p.to_public();
                serde_json::json!({
                    "id": pub_view.id,
                    "name": pub_view.name,
                    "enabled": pub_view.enabled,
                    "adapterAvailable": pub_view.adapter_available,
                })
            })
            .collect();
        let body = serde_json::json!({
            "ok": true,
            "providers": providers,
        });
        let headers = build_cors_headers(request);
        return (
            "HTTP/1.1 200 OK",
            headers,
            body.to_string(),
            RouteAction::None,
        );
    }

    // GET /api/settings — return current provider configs (API keys masked)
    if request.starts_with("GET /api/settings") {
        let config = crate::config::load_config();
        let providers_public: Vec<serde_json::Value> = config
            .downloads
            .providers
            .iter()
            .map(|p| {
                let pub_view = p.to_public();
                serde_json::json!({
                    "id": pub_view.id,
                    "name": pub_view.name,
                    "enabled": pub_view.enabled,
                    "baseUrl": pub_view.base_url,
                    "hasApiKey": pub_view.has_api_key,
                    "keyPreview": pub_view.key_preview,
                    "adapterAvailable": pub_view.adapter_available,
                })
            })
            .collect();
        let body = serde_json::json!({
            "ok": true,
            "providers": providers_public,
            "multiProviderFallback": config.downloads.multi_provider_fallback,
        });
        let headers = build_cors_headers(request);
        return (
            "HTTP/1.1 200 OK",
            headers,
            body.to_string(),
            RouteAction::None,
        );
    }

    // POST /api/settings — update provider configs
    if request.starts_with("POST /api/settings") {
        let body_str = extract_post_body(request);
        match serde_json::from_str::<serde_json::Value>(&body_str) {
            Ok(val) => {
                let mut config = crate::config::load_config().clone();

                // Handle multi-provider fallback toggle
                if let Some(fallback) = val.get("multiProviderFallback").and_then(|v| v.as_bool()) {
                    config.downloads.multi_provider_fallback = fallback;
                    eprintln!("[STEAM_BRIDGE] Multi-provider fallback set to {fallback}");
                }

                // Handle providers array update
                if let Some(providers_arr) = val.get("providers").and_then(|v| v.as_array()) {
                    for incoming_val in providers_arr {
                        let id = match incoming_val.get("id").and_then(|v| v.as_str()) {
                            Some(s) => s.to_string(),
                            None => continue,
                        };
                        let name = incoming_val
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&id)
                            .to_string();
                        let enabled = incoming_val
                            .get("enabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let base_url = incoming_val
                            .get("baseUrl")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let new_api_key = incoming_val
                            .get("apiKey")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        // Merge: only overwrite key if it's a real new value (not a mask)
                        let key_is_masked = new_api_key
                            .as_deref()
                            .map(|k| k.contains('•') || k == "••••" || k.contains("****"))
                            .unwrap_or(false);
                        let key_is_empty = new_api_key
                            .as_deref()
                            .map(|k| k.trim().is_empty())
                            .unwrap_or(true);

                        if let Some(existing) =
                            config.downloads.providers.iter_mut().find(|p| p.id == id)
                        {
                            existing.name = name;
                            existing.enabled = enabled;
                            existing.base_url = base_url;
                            if !key_is_masked && !key_is_empty {
                                existing.api_key = new_api_key;
                            }
                            // If user explicitly cleared the key
                            if incoming_val
                                .get("clearApiKey")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                existing.api_key = None;
                            }
                        } else {
                            config
                                .downloads
                                .providers
                                .push(crate::config::ProviderConfig {
                                    id,
                                    name,
                                    enabled,
                                    base_url,
                                    api_key: if key_is_masked || key_is_empty {
                                        None
                                    } else {
                                        new_api_key
                                    },
                                });
                        }
                    }
                    eprintln!(
                        "[STEAM_BRIDGE] Providers updated ({} entries)",
                        config.downloads.providers.len()
                    );
                }

                let _ = crate::config::save_config(&config);
                let body = serde_json::json!({ "ok": true, "message": "Settings updated." });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 200 OK",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
            Err(_) => {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_JSON",
                    "message": "Request body must be valid JSON.",
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
        }
    }

    // GET /api/local-status/{appId} — check if {appId}.lua exists locally
    if request.starts_with("GET /api/local-status/") {
        if let Some(app_id) = extract_path_param(request, "/api/local-status/") {
            if !is_valid_package_id(app_id) {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_PACKAGE_ID",
                    "message": format!("Invalid app ID '{app_id}'."),
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
            eprintln!("[STEAM_BRIDGE] Local status requested for AppID: {app_id}");
            let in_library = match crate::config::resolve_steam_root() {
                Some(root) => {
                    let result = hubcap::check_local_status(app_id, &root);
                    eprintln!("[STEAM_BRIDGE] Local status result: in_library={result} for AppID={app_id}");
                    result
                }
                None => {
                    eprintln!("[STEAM_BRIDGE] Steam root not resolved, in_library=false for AppID={app_id}");
                    false
                }
            };
            let body = serde_json::json!({
                "ok": true,
                "appId": app_id,
                "inLibrary": in_library,
            });
            let headers = build_cors_headers(request);
            return (
                "HTTP/1.1 200 OK",
                headers,
                body.to_string(),
                RouteAction::None,
            );
        }
    }

    let is_get_download = request.starts_with("GET /api/download-package/");
    let is_post_download = request.starts_with("POST /api/download-package/");

    if is_get_download || is_post_download {
        if let Some(app_id) = extract_path_param(request, "/api/download-package/") {
            if !is_valid_package_id(app_id) {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_PACKAGE_ID",
                    "message": format!("The package identifier '{app_id}' is invalid. Expected a positive numeric ID up to {MAX_PACKAGE_ID_LENGTH} digits."),
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }

            eprintln!("[STEAM_BRIDGE] Download-package request for appId={app_id}");
            let body = serde_json::json!({
                "ok": true,
                "status": "accepted",
                "appId": app_id,
                "message": "Package request added to the LumaForge queue."
            });
            let headers = build_cors_headers(request);
            return (
                "HTTP/1.1 200 OK",
                headers,
                body.to_string(),
                RouteAction::EmitDownloadPackage(app_id.to_string()),
            );
        }
    }

    // GET /api/sources/{appId} — check real provider availability for this app
    if request.starts_with("GET /api/sources/") {
        if let Some(app_id) = extract_path_param(request, "/api/sources/") {
            if !is_valid_package_id(app_id) {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_PACKAGE_ID",
                    "message": format!("Invalid app ID '{app_id}'."),
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
            eprintln!("[STEAM_BRIDGE] Sources availability check for AppID: {app_id}");

            let availability_results = super::providers::check_sources_availability(app_id);

            let sources: Vec<serde_json::Value> = availability_results
                .iter()
                .filter(|availability| availability.available)
                .map(|availability| {
                    serde_json::json!({
                        "id": availability.provider_id,
                        "name": availability.name,
                        "available": true,
                        "selectable": true,
                        "files": availability.file_count,
                        "total": availability.total_size,
                        "detail": availability.detail,
                    })
                })
                .collect();

            let unavailable_sources: Vec<serde_json::Value> = availability_results
                .iter()
                .filter(|availability| !availability.available)
                .map(|availability| {
                    serde_json::json!({
                        "id": availability.provider_id,
                        "name": availability.name,
                        "available": false,
                        "selectable": false,
                        "detail": availability.detail,
                    })
                })
                .collect();

            let message = if sources.is_empty() {
                Some("No package is currently available for this App ID.")
            } else {
                None
            };

            let body = serde_json::json!({
                "ok": true,
                "appId": app_id,
                "sources": sources,
                "unavailableSources": unavailable_sources,
                "message": message,
            });
            let headers = build_cors_headers(request);
            return (
                "HTTP/1.1 200 OK",
                headers,
                body.to_string(),
                RouteAction::None,
            );
        }
    }

    // POST /api/download — download from a provider
    // with fallback, output type, and duplicate protection.
    if request.starts_with("POST /api/download") {
        let body_str = extract_post_body(request);

        match serde_json::from_str::<serde_json::Value>(&body_str) {
            Ok(val) => {
                let app_id = val
                    .get("appId")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");

                let source_id = val
                    .get("sourceId")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");

                let output_type = match val
                    .get("outputType")
                    .and_then(|value| value.as_str())
                    .unwrap_or("lua+manifest")
                {
                    "lua" => "lua".to_string(),
                    "manifest" => "manifest".to_string(),
                    "lua+manifest" => "lua+manifest".to_string(),

                    invalid => {
                        let body = serde_json::json!({
                            "ok": false,
                            "code": "INVALID_OUTPUT_TYPE",
                            "message": format!(
                                "Unsupported output type '{}'. Expected 'lua', 'manifest', or 'lua+manifest'.",
                                invalid
                            ),
                        });

                        let headers = build_cors_headers(request);

                        return (
                            "HTTP/1.1 400 Bad Request",
                            headers,
                            body.to_string(),
                            RouteAction::None,
                        );
                    }
                };

                if !is_valid_package_id(app_id) || source_id.is_empty() {
                    let body = serde_json::json!({
                        "ok": false,
                        "code": "INVALID_PARAMETERS",
                        "message": "Requires valid numeric 'appId' and non-empty 'sourceId'.",
                    });

                    let headers = build_cors_headers(request);

                    return (
                        "HTTP/1.1 400 Bad Request",
                        headers,
                        body.to_string(),
                        RouteAction::None,
                    );
                }

                // Normalize legacy "hubcap" to "hubcapdb"
                let source_id_normalized = match source_id {
                    "hubcap" => "hubcapdb".to_string(),
                    other => other.to_string(),
                };

                // Validate provider exists and is enabled
                {
                    let config = crate::config::load_config();
                    let provider = config
                        .downloads
                        .providers
                        .iter()
                        .find(|p| p.id == source_id_normalized);
                    match provider {
                        None => {
                            let body = serde_json::json!({
                                "ok": false,
                                "code": "SOURCE_DISABLED",
                                "message": format!("Provider '{source_id_normalized}' is not enabled in settings."),
                            });
                            let headers = build_cors_headers(request);
                            return (
                                "HTTP/1.1 400 Bad Request",
                                headers,
                                body.to_string(),
                                RouteAction::None,
                            );
                        }
                        Some(p) if !p.enabled => {
                            let body = serde_json::json!({
                                "ok": false,
                                "code": "SOURCE_DISABLED",
                                "message": format!("Provider '{}' is not enabled in settings.", p.id),
                            });
                            let headers = build_cors_headers(request);
                            return (
                                "HTTP/1.1 400 Bad Request",
                                headers,
                                body.to_string(),
                                RouteAction::None,
                            );
                        }
                        Some(p) => {
                            if super::providers::find_adapter(&p.id).is_none() {
                                let body = serde_json::json!({
                                    "ok": false,
                                    "code": "ADAPTER_UNAVAILABLE",
                                    "message": format!("Provider '{}' does not have a download adapter yet.", p.id),
                                });
                                let headers = build_cors_headers(request);
                                return (
                                    "HTTP/1.1 501 Not Implemented",
                                    headers,
                                    body.to_string(),
                                    RouteAction::None,
                                );
                            }
                        }
                    }
                }

                // Duplicate request protection: check if a job for this appId+source is already active
                {
                    let jobs = job_store().lock().unwrap();
                    for job in jobs.values() {
                        if job.app_id == app_id
                            && job.provider_id == source_id_normalized
                            && (job.status == "queued"
                                || job.status == "validating"
                                || job.status == "checking_availability"
                                || job.status == "downloading"
                                || job.status == "extracting"
                                || job.status == "processing")
                        {
                            let body = serde_json::json!({
                                "ok": false,
                                "code": "DUPLICATE_REQUEST",
                                "message": format!("A download for this app is already in progress (requestId: {}).", job.request_id),
                                "requestId": job.request_id,
                            });
                            let headers = build_cors_headers(request);
                            return (
                                "HTTP/1.1 409 Conflict",
                                headers,
                                body.to_string(),
                                RouteAction::None,
                            );
                        }
                    }
                }

                let request_id = generate_request_id();
                {
                    let mut jobs = job_store().lock().unwrap();
                    jobs.insert(
                        request_id.clone(),
                        DownloadJob {
                            request_id: request_id.clone(),
                            app_id: app_id.to_string(),
                            provider_id: source_id_normalized.clone(),
                            status: "validating".to_string(),
                            progress: None,
                            message: "Validating package request.".to_string(),
                            error_code: None,
                            output_type: Some(output_type.clone()),
                            started_at: Instant::now(),
                            completed_at: None,
                        },
                    );
                }
                eprintln!(
                    "[STEAM_BRIDGE] Download request appId={app_id} source={source_id_normalized} requestId={request_id} outputType={:?}",
                    output_type
                );

                let body = serde_json::json!({
                    "ok": true,
                    "status": "queued",
                    "requestId": request_id,
                    "appId": app_id,
                    "sourceId": source_id_normalized,
                    "outputType": output_type,
                    "message": "Package request added to the LumaForge queue."
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 200 OK",
                    headers,
                    body.to_string(),
                    RouteAction::EmitDownloadProvider {
                        app_id: app_id.to_string(),
                        request_id,
                        provider_id: source_id_normalized,
                    },
                );
            }
            Err(_) => {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_JSON",
                    "message": "Request body must be valid JSON with 'appId' and 'sourceId'.",
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
        }
    }

    // GET /api/download-status/{requestId} — poll job status
    if request.starts_with("GET /api/download-status/") {
        if let Some(request_id) = extract_path_param(request, "/api/download-status/") {
            let jobs = job_store().lock().unwrap();
            if let Some(job) = jobs.get(request_id) {
                let elapsed_ms = job.started_at.elapsed().as_millis() as u64;
                let body = serde_json::json!({
                    "ok": true,
                    "requestId": job.request_id,
                    "appId": job.app_id,
                    "providerId": job.provider_id,
                    "status": job.status,
                    "progress": job.progress,
                    "message": job.message,
                    "errorCode": job.error_code,
                    "elapsedMs": elapsed_ms,
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 200 OK",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            } else {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "UNKNOWN_REQUEST",
                    "message": "No download request found with the given ID."
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 404 Not Found",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
        }
    }

    // POST /api/open-library/{appId} — open Steam Library details page
    if request.starts_with("POST /api/open-library/") {
        if let Some(app_id) = extract_path_param(request, "/api/open-library/") {
            if !is_valid_package_id(app_id) {
                let body = serde_json::json!({
                    "ok": false,
                    "code": "INVALID_PACKAGE_ID",
                    "message": "Invalid app ID."
                });
                let headers = build_cors_headers(request);
                return (
                    "HTTP/1.1 400 Bad Request",
                    headers,
                    body.to_string(),
                    RouteAction::None,
                );
            }
            let uri = format!("steam://nav/games/details/{app_id}");
            eprintln!("[STEAM_BRIDGE] Opening Steam Library details for AppID: {app_id}");
            let body = serde_json::json!({
                "ok": true,
                "appId": app_id,
                "uri": uri,
                "message": "Opening Steam Library."
            });
            let headers = build_cors_headers(request);
            return (
                "HTTP/1.1 200 OK",
                headers,
                body.to_string(),
                RouteAction::OpenLibrary { uri },
            );
        }
    }

    let body = serde_json::json!({
        "ok": false,
        "code": "NOT_FOUND",
        "message": "Endpoint not found.",
        "availableEndpoints": [
            "/health",
            "/api/local-status/{appId}",
            "/api/settings",
            "/api/download-package/{appId}",
            "/api/sources/{appId}",
            "POST /api/download",
            "GET  /api/download-status/{requestId}",
            "POST /api/open-library/{appId}"
        ]
    });
    let headers = build_cors_headers(request);
    (
        "HTTP/1.1 404 Not Found",
        headers,
        body.to_string(),
        RouteAction::None,
    )
}

#[tauri::command]
pub fn get_bridge_status() -> serde_json::Value {
    serde_json::json!({
        "running": BRIDGE_RUNNING.load(Ordering::SeqCst),
        "port": BRIDGE_PORT,
    })
}

fn extract_path_param<'a>(request: &'a str, prefix: &str) -> Option<&'a str> {
    let line = request.lines().next()?;
    let method_end = line.find(' ')?;
    let path_start = method_end + 1;
    let path_end = line[path_start..]
        .find(' ')
        .map(|i| path_start + i)
        .unwrap_or(line.len());
    let path = &line[path_start..path_end];

    let remaining = path.strip_prefix(prefix)?;
    if remaining.is_empty() {
        return None;
    }
    let param = remaining.split('?').next().unwrap_or(remaining);
    let param = param.split('#').next().unwrap_or(param);
    let param = param.trim_end_matches('/');
    if param.is_empty() {
        return None;
    }
    Some(param)
}

fn extract_post_body(request: &str) -> String {
    let header_end = request.find("\r\n\r\n").unwrap_or(request.len());
    let body_start = header_end + 4;
    if body_start >= request.len() {
        return String::new();
    }
    request[body_start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_package_ids() {
        assert!(is_valid_package_id("12345"));
        assert!(is_valid_package_id("1"));
        assert!(is_valid_package_id("999999999999"));
    }

    #[test]
    fn invalid_package_ids() {
        assert!(!is_valid_package_id(""));
        assert!(!is_valid_package_id("0"));
        assert!(!is_valid_package_id("abc"));
        assert!(!is_valid_package_id("12.34"));
        assert!(!is_valid_package_id("-1"));
        assert!(!is_valid_package_id("1234567890123"));
    }

    #[test]
    fn extract_valid_path_param() {
        assert_eq!(
            extract_path_param(
                "GET /api/download-package/12345 HTTP/1.1",
                "/api/download-package/"
            ),
            Some("12345")
        );
        assert_eq!(
            extract_path_param(
                "GET /api/download-package/12345?foo=bar HTTP/1.1",
                "/api/download-package/"
            ),
            Some("12345")
        );
        assert_eq!(
            extract_path_param(
                "GET /api/download-package/ HTTP/1.1",
                "/api/download-package/"
            ),
            None
        );
    }

    #[test]
    fn health_route_200() {
        let req = "GET /health HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("ok"));
    }

    #[test]
    fn invalid_id_returns_400() {
        let req = "GET /api/download-package/abc HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PACKAGE_ID"));
    }

    #[test]
    fn valid_id_returns_200_with_action() {
        let req = "GET /api/download-package/12345 HTTP/1.1\r\n\r\n";
        let (status, _, body, action) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("accepted"));
        assert!(body.contains("12345"));
        assert!(matches!(action, RouteAction::EmitDownloadPackage(ref id) if id == "12345"));
    }

    #[test]
    fn post_download_works() {
        let req = "POST /api/download-package/999 HTTP/1.1\r\n\r\n";
        let (status, _, body, action) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("accepted"));
        assert!(matches!(action, RouteAction::EmitDownloadPackage(ref id) if id == "999"));
    }

    // --- New endpoint tests: download-status and open-library ---

    #[test]
    fn download_status_unknown_request_returns_404() {
        let req = "GET /api/download-status/req-999 HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("404"));
        assert!(body.contains("UNKNOWN_REQUEST"));
    }

    #[test]
    fn download_status_empty_id_returns_404() {
        let req = "GET /api/download-status/ HTTP/1.1\r\n\r\n";
        let (status, _, _, _) = route_request(req);
        assert!(status.contains("404"));
    }

    #[test]
    fn open_library_valid_id_returns_200() {
        let req = "POST /api/open-library/12345 HTTP/1.1\r\n\r\n";
        let (status, _, body, action) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("12345"));
        assert!(body.contains("steam://nav/games/details/12345"));
        assert!(matches!(action, RouteAction::OpenLibrary { .. }));
    }

    #[test]
    fn open_library_invalid_id_returns_400() {
        let req = "POST /api/open-library/abc HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PACKAGE_ID"));
    }

    #[test]
    fn open_library_cors_with_steam_origin() {
        let req = "POST /api/open-library/12345 HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (status, headers, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(body.contains("12345"));
    }

    #[test]
    fn download_status_returns_200_after_download_request() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\",\"sourceId\":\"hubcap\"}";
        let (_, _, body, _) = route_request(req);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let request_id = v["requestId"].as_str().unwrap();

        let status_req = format!("GET /api/download-status/{request_id} HTTP/1.1\r\n\r\n");
        let (status, _, status_body, _) = route_request(&status_req);
        assert!(status.contains("200"));
        let sv: serde_json::Value = serde_json::from_str(&status_body).unwrap();
        assert_eq!(sv["ok"], true);
        assert_eq!(sv["status"], "queued");
        assert_eq!(sv["appId"], "12345");
    }

    #[test]
    fn unsupported_method_returns_404() {
        let req = "DELETE /api/download-package/123 HTTP/1.1\r\n\r\n";
        let (status, _, _, _) = route_request(req);
        assert!(status.contains("404"));
    }

    #[test]
    fn options_returns_204_with_cors() {
        let req = "OPTIONS /api/download-package/123 HTTP/1.1\r\n\r\n";
        let (status, headers, _, _) = route_request(req);
        assert!(status.contains("204"));
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
        assert!(headers.contains("Access-Control-Allow-Methods"));
        assert!(headers.contains("Access-Control-Allow-Headers"));
    }

    #[test]
    fn cors_trusted_origin() {
        let req = "GET /health HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Vary: Origin"));
    }

    #[test]
    fn cors_untrusted_origin_returns_wildcard() {
        let req = "GET /health HTTP/1.1\r\nOrigin: https://evil.com\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
    }

    #[test]
    fn cors_no_origin_header_returns_wildcard() {
        let req = "GET /health HTTP/1.1\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: *"));
    }

    #[test]
    fn cors_null_origin_accepted() {
        let req = "GET /health HTTP/1.1\r\nOrigin: null\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: null"));
    }

    #[test]
    fn post_download_hubcap_normalized_to_hubcapdb() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\",\"sourceId\":\"hubcap\"}";
        let (status, _, body, action) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("queued"));
        assert!(body.contains("hubcapdb"));
        assert!(body.contains("requestId"));
        assert!(matches!(action, RouteAction::EmitDownloadProvider { .. }));
    }

    #[test]
    fn post_download_hubcapdb_direct() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"67890\",\"sourceId\":\"hubcapdb\"}";
        let (status, _, body, action) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("hubcapdb"));
        assert!(body.contains("requestId"));
        assert!(matches!(action, RouteAction::EmitDownloadProvider { .. }));
    }

    #[test]
    fn unknown_route_returns_404() {
        let req = "GET /unknown HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("404"));
        assert!(body.contains("NOT_FOUND"));
    }

    #[test]
    fn zero_id_rejected() {
        let req = "GET /api/download-package/0 HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PACKAGE_ID"));
    }

    #[test]
    fn empty_id_after_prefix_rejected() {
        let req = "GET /api/download-package/ HTTP/1.1\r\n\r\n";
        let (status, _, _, _) = route_request(req);
        assert!(status.contains("404"));
    }

    #[test]
    fn sources_valid_id_returns_200() {
        let req = "GET /api/sources/12345 HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("sources"));
        assert!(body.contains("12345"));
    }

    #[test]
    fn sources_invalid_id_returns_400() {
        let req = "GET /api/sources/abc HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PACKAGE_ID"));
    }

    #[test]
    fn post_download_valid_json_returns_200() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\",\"sourceId\":\"hubcap\"}";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("queued"));
    }

    #[test]
    fn post_download_missing_source_returns_400() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\"}";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PARAMETERS"));
    }

    #[test]
    fn post_download_bad_json_returns_400() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\nnot-json";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_JSON"));
    }

    #[test]
    fn post_download_invalid_appid_returns_400() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"abc\",\"sourceId\":\"hubcap\"}";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PARAMETERS"));
    }

    #[test]
    fn post_download_disabled_source_fails() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\",\"sourceId\":\"ryuu\"}";
        let (status, _, body, _) = route_request(req);
        // ryuu has no adapter — either SOURCE_DISABLED (400) or ADAPTER_UNAVAILABLE (501)
        assert!(
            (status.contains("400") && body.contains("SOURCE_DISABLED"))
                || (status.contains("501") && body.contains("ADAPTER_UNAVAILABLE"))
        );
    }

    #[test]
    fn post_download_unknown_source_returns_400() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"appId\":\"12345\",\"sourceId\":\"nonexistent\"}";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("SOURCE_DISABLED"));
    }

    #[test]
    fn local_status_valid_id_returns_200() {
        let req = "GET /api/local-status/12345 HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("inLibrary"));
        assert!(body.contains("12345"));
    }

    #[test]
    fn local_status_invalid_id_returns_400() {
        let req = "GET /api/local-status/abc HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_PACKAGE_ID"));
    }

    #[test]
    fn get_settings_returns_200() {
        let req = "GET /api/settings HTTP/1.1\r\n\r\n";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("providers"));
        assert!(body.contains("multiProviderFallback"));
    }

    #[test]
    fn post_settings_valid_json_returns_200() {
        let req = "POST /api/settings HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"providers\":[{\"id\":\"hubcap\",\"name\":\"HubcapDB\",\"enabled\":true,\"baseUrl\":\"https://example.com\"}]}";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(body.contains("ok"));
    }

    #[test]
    fn post_settings_bad_json_returns_400() {
        let req = "POST /api/settings HTTP/1.1\r\nContent-Type: application/json\r\n\r\nnot-json";
        let (status, _, body, _) = route_request(req);
        assert!(status.contains("400"));
        assert!(body.contains("INVALID_JSON"));
    }

    #[test]
    fn extract_post_body_empty() {
        let req = "POST /api/download HTTP/1.1\r\n\r\n";
        assert_eq!(extract_post_body(req), "");
    }

    #[test]
    fn extract_post_body_with_json() {
        let req = "POST /api/download HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"key\":\"val\"}";
        assert_eq!(extract_post_body(req), "{\"key\":\"val\"}");
    }

    // --- Tests matching exact runtime request shapes from inject.js ---

    #[test]
    fn sources_with_steam_origin_returns_200_with_cors() {
        let req =
            "GET /api/sources/2141730 HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (status, headers, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(body.contains("sources"));
        assert!(body.contains("2141730"));
    }

    #[test]
    fn local_status_with_steam_origin_returns_200_with_cors() {
        let req = "GET /api/local-status/2141730 HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (status, headers, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(body.contains("inLibrary"));
    }

    #[test]
    fn options_preflight_with_steam_origin() {
        let req = "OPTIONS /api/sources/2141730 HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\nAccess-Control-Request-Method: GET\r\n\r\n";
        let (status, headers, _, _) = route_request(req);
        assert!(status.contains("204"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"));
        assert!(headers.contains("Access-Control-Allow-Headers: Content-Type, Authorization"));
    }

    #[test]
    fn options_preflight_for_download_post() {
        let req = "OPTIONS /api/download HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: Content-Type\r\n\r\n";
        let (status, headers, _, _) = route_request(req);
        assert!(status.contains("204"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"));
    }

    #[test]
    fn cors_case_insensitive_origin() {
        let req = "GET /health HTTP/1.1\r\norigin: https://store.steampowered.com\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
    }

    #[test]
    fn cors_uppercase_origin() {
        let req = "GET /health HTTP/1.1\r\nORIGIN: https://store.steampowered.com\r\n\r\n";
        let (_, headers, _, _) = route_request(req);
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
    }

    // --- STEP 8: /api/providers route tests ---

    #[test]
    fn providers_with_steam_origin_returns_200_with_cors() {
        let req = "GET /api/providers HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (status, headers, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Vary: Origin"));
        assert!(headers.contains("Cache-Control: no-store"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert!(v["providers"].is_array());
    }

    #[test]
    fn providers_no_hubcapdb_duplicates() {
        let req = "GET /api/providers HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let providers = v["providers"].as_array().unwrap();
        let mut seen_ids: Vec<&str> = Vec::new();
        for p in providers {
            let id = p["id"].as_str().unwrap();
            assert!(!seen_ids.contains(&id), "Duplicate provider id: {id}");
            seen_ids.push(id);
        }
    }

    #[test]
    fn providers_no_api_key_exposed() {
        let req = "GET /api/providers HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        assert!(!body.contains("api_key"), "api_key must not be exposed");
        assert!(!body.contains("apiKey"), "apiKey must not be exposed");
        assert!(!body.contains("base_url"), "base_url must not be exposed");
        assert!(!body.contains("baseUrl"), "baseUrl must not be exposed");
        assert!(
            !body.contains("key_preview"),
            "key_preview must not be exposed"
        );
    }

    #[test]
    fn providers_only_enabled_returned() {
        let req = "GET /api/providers HTTP/1.1\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let providers = v["providers"].as_array().unwrap();
        for p in providers {
            assert_eq!(
                p["enabled"], true,
                "Only enabled providers should be returned"
            );
        }
    }

    #[test]
    fn providers_has_hubcapdb_with_adapter() {
        let req = "GET /api/providers HTTP/1.1\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        let providers = v["providers"].as_array().unwrap();
        let hubcapdb = providers.iter().find(|p| p["id"] == "hubcapdb");
        assert!(hubcapdb.is_some(), "HubcapDB must be in provider list");
        assert_eq!(hubcapdb.unwrap()["adapterAvailable"], true);
    }

    #[test]
    fn options_preflight_for_providers() {
        let req = "OPTIONS /api/providers HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\nAccess-Control-Request-Method: GET\r\n\r\n";
        let (status, headers, _, _) = route_request(req);
        assert!(status.contains("204"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"));
        assert!(headers.contains("Access-Control-Allow-Headers: Content-Type, Authorization"));
        assert!(headers.contains("Vary: Origin"));
    }

    // --- STEP 8: local-status contract tests ---

    #[test]
    fn local_status_contract_has_required_fields() {
        let req = "GET /api/local-status/3241660 HTTP/1.1\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["appId"], "3241660");
        assert!(v["inLibrary"].is_boolean(), "inLibrary must be a boolean");
    }

    #[test]
    fn local_status_no_snake_case_fields() {
        let req = "GET /api/local-status/12345 HTTP/1.1\r\n\r\n";
        let (_, _, body, _) = route_request(req);
        assert!(
            body.contains("\"inLibrary\""),
            "Must use camelCase inLibrary"
        );
        assert!(
            !body.contains("\"in_library\""),
            "Must not use snake_case in_library"
        );
        assert!(body.contains("\"appId\""), "Must use camelCase appId");
        assert!(
            !body.contains("\"app_id\""),
            "Must not use snake_case app_id"
        );
    }

    #[test]
    fn local_status_with_steam_origin_cors_contract() {
        let req = "GET /api/local-status/3241660 HTTP/1.1\r\nOrigin: https://store.steampowered.com\r\n\r\n";
        let (status, headers, body, _) = route_request(req);
        assert!(status.contains("200"));
        assert!(headers.contains("Access-Control-Allow-Origin: https://store.steampowered.com"));
        assert!(headers.contains("Vary: Origin"));
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["appId"], "3241660");
    }

    // --- Bridge readiness tests ---

    #[test]
    fn bridge_readiness_becomes_true_after_signal() {
        let flag = &BRIDGE_READY_FLAG;
        let prev = flag.load(Ordering::SeqCst);
        if !prev {
            flag.store(true, Ordering::SeqCst);
            let (lock, cv) = bridge_sync();
            let mut ready = lock.lock().unwrap();
            *ready = true;
            cv.notify_all();
        }
        assert!(wait_until_bridge_ready(100));
        flag.store(prev, Ordering::SeqCst);
    }

    #[test]
    fn wait_until_bridge_ready_returns_true_when_already_ready() {
        assert!(wait_until_bridge_ready(100));
    }

    #[test]
    fn bridge_running_flag_initializes_false() {
        assert!(!BRIDGE_RUNNING.load(Ordering::SeqCst));
    }
}
