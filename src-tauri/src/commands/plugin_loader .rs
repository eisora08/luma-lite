// src-tauri/src/plugin_loader.rs
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub target_url: Option<String>,
    pub requires_react: bool,
    pub enabled: bool,
    #[serde(default = "default_inject_script")]
    pub inject_script: String,
}

fn default_inject_script() -> String {
    "index.js".to_string()
}

#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub code: String,
    pub path: PathBuf,
}

pub fn get_plugins_dir() -> Result<PathBuf, String> {
    let local_app_data = std::env::var("LOCALAPPDATA")
        .map_err(|_| "LOCALAPPDATA not set".to_string())?;
    Ok(PathBuf::from(local_app_data).join("LumaForge").join("plugins"))
}

pub fn load_all_plugins() -> Result<Vec<LoadedPlugin>, String> {
    let plugins_dir = get_plugins_dir()?;
    if !plugins_dir.exists() {
        fs::create_dir_all(&plugins_dir)
            .map_err(|e| format!("Failed to create plugins dir: {}", e))?;
        return Ok(Vec::new());
    }

    let mut loaded = Vec::new();
    for entry in fs::read_dir(&plugins_dir).map_err(|e| format!("Failed to read plugins dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("manifest.json");
        if !manifest_path.exists() {
            continue;
        }

        let manifest_content = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("Failed to read manifest {}: {}", manifest_path.display(), e))?;
        let manifest: PluginManifest = serde_json::from_str(&manifest_content)
            .map_err(|e| format!("Failed to parse manifest {}: {}", manifest_path.display(), e))?;

        let code_path = path.join(&manifest.inject_script);
        if !code_path.exists() {
            continue;
        }
        let code = fs::read_to_string(&code_path)
            .map_err(|e| format!("Failed to read code {}: {}", code_path.display(), e))?;

        loaded.push(LoadedPlugin {
            manifest,
            code,
            path,
        });
    }

    Ok(loaded)
}

pub fn load_enabled_plugins() -> Result<Vec<LoadedPlugin>, String> {
    let all = load_all_plugins()?;
    let enabled: Vec<LoadedPlugin> = all.into_iter()
        .filter(|p| p.manifest.enabled)
        .collect();
    Ok(enabled)
}