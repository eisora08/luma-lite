use std::fs;
use std::sync::OnceLock;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub source: String,
    pub has_detect: bool,
    pub script_path: Option<String>,
    pub manifest_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cef_injection: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inject_script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestActivation {
    #[serde(rename = "cefInjection", default)]
    cef_injection: bool,
    #[serde(rename = "injectScript")]
    inject_script: Option<String>,
    #[serde(rename = "targetUrl")]
    target_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginManifest {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    #[serde(default)]
    has_detect: bool,
    activation: Option<ManifestActivation>,
}

#[derive(Debug, Deserialize)]
struct ExtensionConfig {
    #[serde(default)]
    enabled: bool,
}

// ---------------------------------------------------------------------------
// Plugin cache
// ---------------------------------------------------------------------------

static PLUGINS_CACHE: OnceLock<DashMap<String, PluginEntry>> = OnceLock::new();

pub(crate) fn get_plugins_cache() -> &'static DashMap<String, PluginEntry> {
    PLUGINS_CACHE.get_or_init(DashMap::new)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_plugin_enabled(config_path: &std::path::Path) -> bool {
    if !config_path.exists() {
        return false;
    }

    fs::read_to_string(config_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ExtensionConfig>(&raw).ok())
        .map(|config| config.enabled)
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Internal scan (used by both the Tauri command and the boot setup)
// ---------------------------------------------------------------------------

/// Scan the plugins directory and return all discovered plugins.
/// When `app_handle` is Some, emits a `"plugins-scanned"` event.
pub fn do_scan_plugins(app_handle: Option<&tauri::AppHandle>) -> Result<Vec<PluginEntry>, String> {
    let plugins_dir = crate::config::resolve_plugins_dir();
    if !plugins_dir.exists() {
        return Ok(vec![]);
    }

    let dir_entries =
        fs::read_dir(&plugins_dir).map_err(|e| format!("Failed to read plugins dir: {e}"))?;

    let mut plugins: Vec<PluginEntry> = Vec::new();

    for entry in dir_entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }

        let plugin_dir = entry.path();
        let manifest_path = plugin_dir.join("manifest.json");
        let script_path = plugin_dir.join("extension.lua");
        let config_path = plugin_dir.join("extension-config.json");

        if !manifest_path.exists() {
            continue;
        }

        let manifest_raw = match fs::read_to_string(&manifest_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match serde_json::from_str(&manifest_raw) {
            Ok(m) => m,
            Err(e) => {
                eprintln!(
                    "[PLUGINS] Failed to parse manifest {}: {e}",
                    manifest_path.display()
                );
                continue;
            }
        };

        let id = manifest
            .id
            .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());

        let enabled = read_plugin_enabled(&config_path);

        let (cef_injection, inject_script, target_url) = match &manifest.activation {
            Some(a) => (
                Some(a.cef_injection),
                a.inject_script.clone(),
                a.target_url.clone(),
            ),
            None => (None, None, None),
        };

        if id == "steam-store-helper" {
            eprintln!(
                "[PLUGIN_DIAG] Scanned '{}': enabled={}, cef_injection={:?}, inject_script={:?}, target_url={:?}, manifest={}",
                id, enabled, cef_injection, inject_script, target_url, manifest_path.display()
            );
        }

        plugins.push(PluginEntry {
            id: id.clone(),
            name: manifest.name.unwrap_or_else(|| id.clone()),
            version: manifest.version.unwrap_or_else(|| "0.0.0".into()),
            description: manifest.description.unwrap_or_default(),
            author: manifest.author.unwrap_or_default(),
            enabled,
            source: "local".into(),
            has_detect: manifest.has_detect,
            script_path: if script_path.exists() {
                Some(script_path.to_string_lossy().to_string())
            } else {
                None
            },
            manifest_path: Some(manifest_path.to_string_lossy().to_string()),
            cef_injection,
            inject_script,
            target_url,
        });
    }

    let cache = get_plugins_cache();
    cache.clear();
    for plugin in &plugins {
        cache.insert(plugin.id.clone(), plugin.clone());
    }

    if let Some(handle) = app_handle {
        let _ = handle.emit("plugins-scanned", &plugins);
    }

    eprintln!(
        "[PLUGINS] Scanned {} plugins from {}",
        plugins.len(),
        plugins_dir.display()
    );

    Ok(plugins)
}

fn build_plugin_from_disk(extension_id: &str) -> Option<PluginEntry> {
    let plugins_dir = crate::config::resolve_plugins_dir();
    let plugin_dir = plugins_dir.join(extension_id);
    let manifest_path = plugin_dir.join("manifest.json");
    let script_path = plugin_dir.join("extension.lua");
    let config_path = plugin_dir.join("extension-config.json");
    if !manifest_path.exists() {
        return None;
    }
    let raw = fs::read_to_string(&manifest_path).ok()?;
    let m: PluginManifest = serde_json::from_str(&raw).ok()?;

    let (cef_injection, inject_script, target_url) = match &m.activation {
        Some(a) => (
            Some(a.cef_injection),
            a.inject_script.clone(),
            a.target_url.clone(),
        ),
        None => (None, None, None),
    };

    Some(PluginEntry {
        id: extension_id.to_string(),
        name: m.name.unwrap_or_else(|| extension_id.to_string()),
        version: m.version.unwrap_or_else(|| "0.0.0".into()),
        description: m.description.unwrap_or_default(),
        author: m.author.unwrap_or_default(),
        enabled: read_plugin_enabled(&config_path),
        source: "local".into(),
        has_detect: m.has_detect,
        script_path: if script_path.exists() {
            Some(script_path.to_string_lossy().to_string())
        } else {
            None
        },
        manifest_path: Some(manifest_path.to_string_lossy().to_string()),
        cef_injection,
        inject_script,
        target_url,
    })
}

// ---------------------------------------------------------------------------
// CEF injection pipeline (called from toggle_plugin)
// ---------------------------------------------------------------------------

fn activate_cef_injection(plugin: &PluginEntry) {
    if plugin.cef_injection != Some(true) {
        eprintln!(
            "[PLUGINS] Skipping CEF injection for '{}': cef_injection={:?}",
            plugin.id, plugin.cef_injection
        );
        return;
    }

    let script_file = match &plugin.inject_script {
        Some(script_file) if !script_file.trim().is_empty() => script_file.clone(),
        _ => {
            eprintln!(
                "[PLUGINS] Skipping CEF injection for '{}': inject_script is missing",
                plugin.id
            );
            return;
        }
    };

    let target = plugin
        .target_url
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "store.steampowered.com".to_string());

    let script_path = match crate::config::resolve_inject_script(&plugin.id, &script_file) {
        Some(path) => path,
        None => {
            eprintln!(
                "[PLUGINS] Cannot resolve inject script '{}' for '{}'",
                script_file, plugin.id
            );
            return;
        }
    };

    let js_code = match fs::read_to_string(&script_path) {
        Ok(code) => code,
        Err(error) => {
            eprintln!(
                "[PLUGINS] Failed to read inject script {}: {error}",
                script_path.display()
            );
            return;
        }
    };

    eprintln!(
        "[PLUGINS] Activating CEF injection for '{}' (target: '{}', script: {})",
        plugin.id,
        target,
        script_path.display()
    );

    let skip = super::steam_inject::get_injected_target_ids(&plugin.id);
    let result = super::steam_inject::inject_code_into_tabs(&target, &js_code, &skip);

    if result.success && !result.injected_tab_urls.is_empty() {
        super::steam_inject::track_injection(
            &plugin.id,
            &target,
            &result.injected_target_ids,
            &result.injected_tab_urls,
            &std::iter::repeat(None)
                .take(result.injected_target_ids.len())
                .collect::<Vec<_>>(),
            &std::iter::repeat(None)
                .take(result.injected_target_ids.len())
                .collect::<Vec<_>>(),
        );

        eprintln!(
            "[PLUGINS] Injected {} into {} tab(s) for '{}'",
            script_file,
            result.injected_tab_urls.len(),
            plugin.id
        );
    } else if let Some(error) = result.error.as_ref() {
        eprintln!("[PLUGINS] CEF injection for '{}': {error}", plugin.id);
    } else {
        eprintln!(
            "[PLUGINS] CEF injection for '{}': no matching tabs found ({} total tabs)",
            plugin.id, result.tabs_found
        );
    }

    super::steam_inject::start_target_monitor(plugin.id.clone(), target);
}

fn deactivate_cef_injection(plugin_id: &str) {
    super::steam_inject::clear_injection(plugin_id);
    super::steam_inject::stop_target_monitor();
    eprintln!("[PLUGINS] Cleared CEF injection state for {plugin_id}");
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPluginsResult {
    pub plugins: Vec<PluginEntry>,
    pub plugins_path: String,
}

#[tauri::command]
pub fn list_plugins() -> Result<ListPluginsResult, String> {
    let plugins = do_scan_plugins(None)?;
    let plugins_path = crate::config::resolve_plugins_dir()
        .to_string_lossy()
        .to_string();
    Ok(ListPluginsResult {
        plugins,
        plugins_path,
    })
}

#[tauri::command]
pub fn scan_plugins() -> Result<Vec<PluginEntry>, String> {
    do_scan_plugins(None)
}

#[tauri::command]
pub fn toggle_plugin(
    extension_id: String,
    enabled: bool,
    app_handle: tauri::AppHandle,
) -> Result<PluginEntry, String> {
    let cache = get_plugins_cache();
    let mut plugin = cache
        .get(&extension_id)
        .map(|r| r.clone())
        .or_else(|| build_plugin_from_disk(&extension_id))
        .ok_or_else(|| format!("Plugin {extension_id} not found"))?;

    // Write extension-config.json
    let plugins_dir = crate::config::resolve_plugins_dir();
    let config_path = plugins_dir
        .join(&extension_id)
        .join("extension-config.json");

    if let Some(parent) = config_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create plugin dir: {e}"))?;
        }
    }

    let config = serde_json::json!({ "enabled": enabled });
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).unwrap_or_default(),
    )
    .map_err(|e| format!("Failed to write config: {e}"))?;

    plugin.enabled = enabled;
    cache.insert(extension_id.clone(), plugin.clone());

    // ------------------------------------------------------------------
    // Invoke Lua lifecycle hooks
    // ------------------------------------------------------------------
    if let Some(ref script_path) = plugin.script_path {
        let install_dir = crate::config::resolve_steam_root()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        match super::extension_lifecycle::load_extension(extension_id.clone(), script_path.clone())
        {
            Ok(_) => {
                let hook = if enabled { "enable" } else { "disable" };
                let result = if enabled {
                    super::extension_lifecycle::call_extension_enable(
                        extension_id.clone(),
                        install_dir,
                    )
                } else {
                    super::extension_lifecycle::call_extension_disable(
                        extension_id.clone(),
                        install_dir,
                    )
                };
                match result {
                    Ok(val) => {
                        eprintln!("[PLUGINS] Lua extension.{hook}({extension_id}) returned: {val}");
                    }
                    Err(e) => {
                        eprintln!("[PLUGINS] Lua extension.{hook}({extension_id}) failed: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("[PLUGINS] Failed to load Lua script for {extension_id}: {e}");
            }
        }
    }

    // ------------------------------------------------------------------
    // CEF injection lifecycle
    // ------------------------------------------------------------------
    eprintln!(
        "[PLUGIN_DIAG] Before CEF lifecycle '{}': enabled={}, cef_injection={:?}, inject_script={:?}, target_url={:?}",
        plugin.id, enabled, plugin.cef_injection, plugin.inject_script, plugin.target_url
    );

    if enabled {
        activate_cef_injection(&plugin);
    } else {
        deactivate_cef_injection(&extension_id);
    }

    let _ = app_handle.emit("plugin-toggled", &plugin);

    eprintln!(
        "[PLUGINS] Plugin {} {}",
        extension_id,
        if enabled { "enabled" } else { "disabled" }
    );

    Ok(plugin)
}

#[tauri::command]
pub fn reload_plugins(app_handle: tauri::AppHandle) -> Result<Vec<PluginEntry>, String> {
    crate::commands::extension_lifecycle::clear_engine_cache();
    do_scan_plugins(Some(&app_handle))
}
