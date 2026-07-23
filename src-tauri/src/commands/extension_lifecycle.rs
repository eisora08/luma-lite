use std::fs;
use std::sync::OnceLock;

use dashmap::DashMap;

use crate::lua_engine::{LuaEngine, LuaEngineConfig};

// ---------------------------------------------------------------------------
// Engine cache — one LuaEngine per extension_id, lives for the session
// ---------------------------------------------------------------------------

static ENGINES: OnceLock<DashMap<String, LuaEngine>> = OnceLock::new();

fn get_engines() -> &'static DashMap<String, LuaEngine> {
    ENGINES.get_or_init(DashMap::new)
}

/// Clear all cached Lua engines. Called by `reload_plugins`.
pub fn clear_engine_cache() {
    get_engines().clear();
    eprintln!("[LUA_ENGINE] Engine cache cleared");
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn load_extension(
    extension_id: String,
    script_path: String,
) -> Result<serde_json::Value, String> {
    eprintln!("[LIFECYCLE] load_extension({extension_id}, script_path='{script_path}')");
    let script = fs::read_to_string(&script_path)
        .map_err(|e| format!("Failed to read script {script_path}: {e}"))?;
    eprintln!("[LIFECYCLE] Read {} bytes from {script_path}", script.len());

    let mut engine = LuaEngine::new(LuaEngineConfig::default())
        .map_err(|e| format!("Engine init failed: {e}"))?;

    let table = engine
        .load_and_evaluate(&extension_id, &script)
        .map_err(|e| format!("Extension load failed for {extension_id}: {e}"))?;

    let value = serde_json::to_value(&table).map_err(|e| format!("Serialization error: {e}"))?;

    get_engines().insert(extension_id.clone(), engine);
    eprintln!("[LIFECYCLE] Loaded extension: {extension_id}, meta={value}");

    Ok(value)
}

#[tauri::command]
pub fn call_extension_detect(
    extension_id: String,
    install_dir: String,
) -> Result<serde_json::Value, String> {
    let engines = get_engines();
    let engine = engines.get(&extension_id).ok_or_else(|| {
        format!("Extension {extension_id} not loaded — call load_extension first")
    })?;
    let result = engine.call_function("detect", &install_dir)?;
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| "detect() returned success=false".to_string()));
    }
    Ok(result.value.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub fn call_extension_install(
    extension_id: String,
    install_dir: String,
) -> Result<serde_json::Value, String> {
    eprintln!("[LIFECYCLE] call_extension_install({extension_id}, install_dir='{install_dir}')");
    let engines = get_engines();
    let engine = engines
        .get(&extension_id)
        .ok_or_else(|| format!("Extension {extension_id} not loaded"))?;
    let result = engine.call_function("install", &install_dir)?;
    eprintln!(
        "[LIFECYCLE] install result: success={}, value={:?}, error={:?}",
        result.success, result.value, result.error
    );
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| "install() returned success=false".to_string()));
    }
    Ok(result.value.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub fn call_extension_enable(
    extension_id: String,
    install_dir: String,
) -> Result<serde_json::Value, String> {
    let engines = get_engines();
    let engine = engines
        .get(&extension_id)
        .ok_or_else(|| format!("Extension {extension_id} not loaded"))?;
    let result = engine.call_function("enable", &install_dir)?;
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| "enable() returned success=false".to_string()));
    }
    Ok(result.value.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub fn call_extension_disable(
    extension_id: String,
    install_dir: String,
) -> Result<serde_json::Value, String> {
    let engines = get_engines();
    let engine = engines
        .get(&extension_id)
        .ok_or_else(|| format!("Extension {extension_id} not loaded"))?;
    let result = engine.call_function("disable", &install_dir)?;
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| "disable() returned success=false".to_string()));
    }
    Ok(result.value.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub fn call_extension_uninstall(
    extension_id: String,
    install_dir: String,
) -> Result<serde_json::Value, String> {
    let engines = get_engines();
    let engine = engines
        .get(&extension_id)
        .ok_or_else(|| format!("Extension {extension_id} not loaded"))?;
    let result = engine.call_function("uninstall", &install_dir)?;
    if !result.success {
        return Err(result
            .error
            .unwrap_or_else(|| "uninstall() returned success=false".to_string()));
    }
    Ok(result.value.unwrap_or(serde_json::Value::Null))
}

#[tauri::command]
pub fn read_extension_text_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("Failed to read {path}: {e}"))
}
