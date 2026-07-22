use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};
use tauri_plugin_shell::ShellExt;

const STEAM_EXIT_TIMEOUT_SECS: u64 = 30;
const CEF_START_TIMEOUT_SECS: u64 = 45;
const PROCESS_POLL_INTERVAL_MS: u64 = 500;
const CEF_POLL_INTERVAL_MS: u64 = 750;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamRuntimeStatus {
    pub steam_running: bool,
    pub cef_debugging_enabled: bool,
    pub cef_debug_port: Option<u16>,
    pub restart_required: bool,
    pub steam_executable_found: bool,
    pub steam_executable: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamRuntimeOperationResult {
    pub ok: bool,
    pub status: String,
    pub message: String,
    pub steam_running: bool,
    pub cef_debugging_enabled: bool,
    pub cef_debug_port: Option<u16>,
}

fn resolve_steam_executable() -> Option<PathBuf> {
    let steam_root = crate::config::resolve_steam_root()?;
    let executable = steam_root.join("steam.exe");

    if executable.is_file() {
        Some(executable)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn is_steam_running() -> bool {
    use std::os::windows::process::CommandExt;

    let output = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq steam.exe", "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let output = match output {
        Ok(output) => output,
        Err(error) => {
            eprintln!("[STEAM_RUNTIME] Failed to query Steam process: {error}");

            return false;
        }
    };

    if !output.status.success() {
        eprintln!(
            "[STEAM_RUNTIME] tasklist returned status: {}",
            output.status
        );

        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    stdout.lines().any(|line| {
        line.trim_start()
            .to_ascii_lowercase()
            .starts_with("\"steam.exe\"")
    })
}

#[cfg(not(target_os = "windows"))]
fn is_steam_running() -> bool {
    false
}

fn current_runtime_status() -> SteamRuntimeStatus {
    let cef_debug_port = super::steam_inject::detect_cef_debug_port_silent();

    let steam_running = is_steam_running();

    let steam_executable = resolve_steam_executable();

    SteamRuntimeStatus {
        steam_running,
        cef_debugging_enabled: cef_debug_port.is_some(),
        cef_debug_port,
        restart_required: steam_running && cef_debug_port.is_none(),
        steam_executable_found: steam_executable.is_some(),
        steam_executable: steam_executable.map(|path| path.to_string_lossy().to_string()),
    }
}

fn operation_result(
    ok: bool,
    status: impl Into<String>,
    message: impl Into<String>,
) -> SteamRuntimeOperationResult {
    let runtime = current_runtime_status();

    SteamRuntimeOperationResult {
        ok,
        status: status.into(),
        message: message.into(),
        steam_running: runtime.steam_running,
        cef_debugging_enabled: runtime.cef_debugging_enabled,
        cef_debug_port: runtime.cef_debug_port,
    }
}

#[cfg(target_os = "windows")]
fn launch_steam_process_with_cef() -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    if super::steam_inject::detect_cef_debug_port().is_some() {
        eprintln!("[STEAM_RUNTIME] Steam CEF debugging is already active");

        return Ok(());
    }

    if is_steam_running() {
        return Err(
            "Steam is already running without CEF debugging. A restart is required.".to_string(),
        );
    }

    let executable = resolve_steam_executable()
        .ok_or_else(|| "Steam executable could not be found.".to_string())?;

    eprintln!(
        "[STEAM_RUNTIME] Starting Steam with CEF debugging: {}",
        executable.display()
    );

    Command::new(&executable)
        .arg("-cef-enable-debugging")
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| format!("Failed to start Steam with CEF debugging: {error}"))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn launch_steam_process_with_cef() -> Result<(), String> {
    Err("Automatic Steam CEF startup is currently supported only on Windows.".to_string())
}

fn wait_for_steam_exit(timeout: Duration) -> Result<(), String> {
    let started_at = Instant::now();
    let mut last_logged_second = 0;

    while started_at.elapsed() < timeout {
        if !is_steam_running() {
            eprintln!(
                "[STEAM_RUNTIME] Steam process exited normally after {} second(s)",
                started_at.elapsed().as_secs()
            );

            return Ok(());
        }

        let elapsed_seconds = started_at.elapsed().as_secs();

        if elapsed_seconds != last_logged_second {
            last_logged_second = elapsed_seconds;

            eprintln!(
                "[STEAM_RUNTIME] Waiting for Steam to exit: {}s / {}s",
                elapsed_seconds,
                timeout.as_secs()
            );
        }

        thread::sleep(Duration::from_millis(PROCESS_POLL_INTERVAL_MS));
    }

    Err(format!(
        "Steam did not close within {} seconds. Close Steam manually and try again.",
        timeout.as_secs()
    ))
}

fn wait_for_cef_debugger(timeout: Duration) -> Result<u16, String> {
    let started_at = Instant::now();

    while started_at.elapsed() < timeout {
        if let Some(port) = super::steam_inject::detect_cef_debug_port() {
            eprintln!("[STEAM_RUNTIME] Steam CEF debugger ready on port {port}");

            return Ok(port);
        }

        thread::sleep(Duration::from_millis(CEF_POLL_INTERVAL_MS));
    }

    Err(format!(
        "Steam started, but CEF debugging did not become available within {} seconds.",
        timeout.as_secs()
    ))
}

fn request_normal_steam_exit(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if !is_steam_running() {
        return Ok(());
    }

    eprintln!("[STEAM_RUNTIME] Requesting normal Steam shutdown");

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        if let Some(steam_executable) = resolve_steam_executable() {
            eprintln!(
                "[STEAM_RUNTIME] Requesting shutdown through Steam executable: {}",
                steam_executable.display()
            );

            match Command::new(&steam_executable)
                .arg("-shutdown")
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
            {
                Ok(_) => {
                    return Ok(());
                }

                Err(error) => {
                    eprintln!("[STEAM_RUNTIME] Steam executable shutdown request failed: {error}");
                }
            }
        }
    }

    eprintln!("[STEAM_RUNTIME] Falling back to steam://exit");

    app_handle
        .shell()
        .open("steam://exit", None)
        .map_err(|error| format!("Failed to request Steam shutdown: {error}"))
}

fn start_steam_and_wait_for_cef() -> SteamRuntimeOperationResult {
    if let Some(port) = super::steam_inject::detect_cef_debug_port() {
        return operation_result(
            true,
            "ready",
            format!("Steam CEF debugging is already active on port {port}."),
        );
    }

    if is_steam_running() {
        return operation_result(
            false,
            "restart_required",
            "Steam is running without CEF debugging. Restart Steam to enable Store integration.",
        );
    }

    if resolve_steam_executable().is_none() {
        return operation_result(
            false,
            "steam_not_found",
            "Steam executable could not be found.",
        );
    }

    if let Err(error) = launch_steam_process_with_cef() {
        return operation_result(false, "start_failed", error);
    }

    match wait_for_cef_debugger(Duration::from_secs(CEF_START_TIMEOUT_SECS)) {
        Ok(port) => operation_result(
            true,
            "ready",
            format!("Steam started with CEF debugging on port {port}."),
        ),

        Err(error) => operation_result(false, "cef_start_timeout", error),
    }
}

fn restart_steam_and_wait_for_cef(app_handle: tauri::AppHandle) -> SteamRuntimeOperationResult {
    if let Some(port) = super::steam_inject::detect_cef_debug_port() {
        return operation_result(
            true,
            "ready",
            format!("Steam CEF debugging is already active on port {port}."),
        );
    }

    if resolve_steam_executable().is_none() {
        return operation_result(
            false,
            "steam_not_found",
            "Steam executable could not be found.",
        );
    }

    if is_steam_running() {
        if let Err(error) = request_normal_steam_exit(&app_handle) {
            return operation_result(false, "exit_request_failed", error);
        }

        if let Err(error) = wait_for_steam_exit(Duration::from_secs(STEAM_EXIT_TIMEOUT_SECS)) {
            return operation_result(false, "steam_exit_timeout", error);
        }
    }

    if let Err(error) = launch_steam_process_with_cef() {
        return operation_result(false, "start_failed", error);
    }

    match wait_for_cef_debugger(Duration::from_secs(CEF_START_TIMEOUT_SECS)) {
        Ok(port) => operation_result(
            true,
            "ready",
            format!("Steam restarted with CEF debugging on port {port}."),
        ),

        Err(error) => operation_result(false, "cef_start_timeout", error),
    }
}

#[tauri::command]
pub async fn get_steam_runtime_status() -> Result<SteamRuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(current_runtime_status)
        .await
        .map_err(|error| format!("Steam runtime status task failed: {error}"))
}

#[tauri::command]
pub async fn start_steam_with_cef() -> Result<SteamRuntimeOperationResult, String> {
    tauri::async_runtime::spawn_blocking(start_steam_and_wait_for_cef)
        .await
        .map_err(|error| format!("Steam startup task failed: {error}"))
}

#[tauri::command]
pub async fn restart_steam_with_cef(
    app_handle: tauri::AppHandle,
) -> Result<SteamRuntimeOperationResult, String> {
    tauri::async_runtime::spawn_blocking(move || restart_steam_and_wait_for_cef(app_handle))
        .await
        .map_err(|error| format!("Steam restart task failed: {error}"))
}

#[tauri::command]
pub fn request_steam_shutdown(
    app_handle: tauri::AppHandle,
) -> Result<SteamRuntimeOperationResult, String> {
    request_normal_steam_exit(&app_handle)?;

    Ok(operation_result(
        true,
        "exit_requested",
        "Steam shutdown was requested.",
    ))
}
