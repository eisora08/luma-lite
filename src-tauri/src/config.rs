use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppearanceSettings {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_surface")]
    pub surface_style: String,
    #[serde(default = "default_density")]
    pub density: String,
    #[serde(default)]
    pub reduce_motion: bool,
}

fn default_theme() -> String {
    "midnight-blue".into()
}
fn default_surface() -> String {
    "tinted".into()
}
fn default_density() -> String {
    "comfortable".into()
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            surface_style: default_surface(),
            density: default_density(),
            reduce_motion: false,
        }
    }
}

/// A download provider configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    /// Unique identifier: "hubcapdb", "ryuu", "twentytwo", "sushi", "custom"
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether this provider is enabled.
    pub enabled: bool,
    /// Base URL for the provider API.
    pub base_url: String,
    /// Optional API key (saved in plaintext on disk, masked when sent to frontend).
    #[serde(default)]
    pub api_key: Option<String>,
}

/// Public representation sent to the frontend (API keys masked).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigPublic {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub base_url: String,
    /// `true` if an API key is configured (never reveal the actual value).
    pub has_api_key: bool,
    /// Masked preview like `sk-a...xYz`.
    pub key_preview: String,
    /// Whether this provider has a working adapter in the backend.
    pub adapter_available: bool,
}

impl ProviderConfig {
    /// Mask the API key for frontend display.
    pub fn to_public(&self) -> ProviderConfigPublic {
        let (has_key, preview) = match &self.api_key {
            Some(k) if !k.is_empty() => {
                let preview = if k.len() > 8 {
                    format!("{}...{}", &k[..4], &k[k.len() - 4..])
                } else {
                    "••••".to_string()
                };
                (true, preview)
            }
            _ => (false, String::new()),
        };
        let adapter_available = matches!(self.id.as_str(), "hubcapdb");
        ProviderConfigPublic {
            id: self.id.clone(),
            name: self.name.clone(),
            enabled: self.enabled,
            base_url: self.base_url.clone(),
            has_api_key: has_key,
            key_preview: preview,
            adapter_available,
        }
    }
}

/// Downloads configuration section, nested inside AppConfig.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsConfig {
    /// If true, try the next enabled provider when one fails.
    #[serde(default = "default_multi_provider_fallback")]
    pub multi_provider_fallback: bool,
    /// Ordered list of download providers.
    #[serde(default = "default_providers")]
    pub providers: Vec<ProviderConfig>,
}

fn default_multi_provider_fallback() -> bool {
    true
}

impl Default for DownloadsConfig {
    fn default() -> Self {
        Self {
            multi_provider_fallback: default_multi_provider_fallback(),
            providers: default_providers(),
        }
    }
}

/// Return the default set of providers.
fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "hubcapdb".into(),
            name: "HubcapDB".into(),
            enabled: true,
            base_url: "https://hubcapmanifest.com".into(),
            api_key: Some(String::new()),
        },
        ProviderConfig {
            id: "ryuu".into(),
            name: "Ryuu".into(),
            enabled: false,
            base_url: "https://generator.ryuu.lol".into(),
            api_key: Some(String::new()),
        },
        ProviderConfig {
            id: "twentytwo".into(),
            name: "TwentyTwo Cloud".into(),
            enabled: false,
            base_url: "https://api.twentytwocloud.com".into(),
            api_key: None,
        },
        ProviderConfig {
            id: "sushi".into(),
            name: "Sushi".into(),
            enabled: false,
            base_url: "https://raw.githubusercontent.com/sushi-dev55-alt/sushitools-games-repo-alt"
                .into(),
            api_key: None,
        },
        ProviderConfig {
            id: "custom".into(),
            name: "Custom API".into(),
            enabled: false,
            base_url: "https://api.example.com".into(),
            api_key: None,
        },
    ]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// User-configured Steam root directory.
    /// `None` means "use auto-detected path".
    pub steam_root: Option<String>,
    /// User appearance preferences.
    #[serde(default)]
    pub appearance: AppearanceSettings,
    /// Download provider configurations.
    #[serde(default)]
    pub downloads: DownloadsConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamRootInfo {
    /// The resolved path that will actually be used.
    pub resolved_path: Option<String>,
    /// `true` if the path came from user config rather than auto-detection.
    pub is_custom: bool,
    /// Where the config file lives on disk.
    pub config_path: String,
}

// ---------------------------------------------------------------------------
// Config file persistence
// ---------------------------------------------------------------------------

static CONFIG: OnceLock<AppConfig> = OnceLock::new();

/// Canonical application data directory: `{local_data_dir}/LumaForge`
fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LumaForge")
}

/// Resolve the config file path: `{app_data_dir}/config.json`
fn config_path() -> PathBuf {
    app_data_dir().join("config.json")
}

/// Load the app config from disk, run idempotent migration, and cache.
pub fn load_config() -> &'static AppConfig {
    CONFIG.get_or_init(|| {
        let path = config_path();
        let mut config: AppConfig = match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => AppConfig::default(),
        };

        let migrated = migrate_providers(&mut config.downloads.providers);
        if migrated {
            eprintln!("[CONFIG] Provider migration applied, saving.");
            if let Err(e) = save_config(&config) {
                eprintln!("[CONFIG] Failed to save migrated config: {e}");
            }
        }

        config
    })
}

/// Save the app config to disk.
pub(crate) fn save_config(config: &AppConfig) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {e}"))?;
    }
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;
    fs::write(&path, &json).map_err(|e| format!("Failed to write config: {e}"))?;
    eprintln!("[CONFIG] Saved config to {}", path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Provider migration (idempotent)
// ---------------------------------------------------------------------------

/// Canonical provider IDs in display order.
const CANONICAL_PROVIDER_IDS: &[&str] = &["hubcapdb", "ryuu", "twentytwo", "sushi", "custom"];

/// Legacy ID that must be merged into its canonical replacement.
const LEGACY_ID_HUBCAP: &str = "hubcap";

/// Run idempotent provider migration on the given list:
/// 1. Merge any legacy "hubcap" entry into "hubcapdb" (preserving user values).
/// 2. Remove duplicate IDs (keep first occurrence with the most non-empty values).
/// 3. Reorder to canonical order, appending any unknown IDs at the end.
/// Returns `true` if the list was modified.
fn migrate_providers(providers: &mut Vec<ProviderConfig>) -> bool {
    let original_len = providers.len();
    let mut hubcapdb_idx: Option<usize> = None;
    let mut hubcap_legacy_idx: Option<usize> = None;

    // Find indices
    for (i, p) in providers.iter().enumerate() {
        if p.id == "hubcapdb" && hubcapdb_idx.is_none() {
            hubcapdb_idx = Some(i);
        }
        if p.id == LEGACY_ID_HUBCAP && hubcap_legacy_idx.is_none() {
            hubcap_legacy_idx = Some(i);
        }
    }

    // Merge legacy "hubcap" into "hubcapdb"
    if let (Some(legacy_idx), Some(db_idx)) = (hubcap_legacy_idx, hubcapdb_idx) {
        let legacy = providers.remove(legacy_idx);
        // After remove, indices shift. If legacy was before db, db_idx decreases.
        let adjusted_db_idx = if legacy_idx < db_idx {
            db_idx - 1
        } else {
            db_idx
        };
        let db = &mut providers[adjusted_db_idx];

        // Prefer legacy values only if db has empty/default values
        if db.base_url.is_empty() || db.base_url == "https://example.com" {
            if !legacy.base_url.is_empty() && legacy.base_url != "https://example.com" {
                db.base_url = legacy.base_url;
            }
        }
        if db.api_key.as_deref().map_or(true, |k| k.is_empty()) {
            if legacy.api_key.as_ref().map_or(false, |k| !k.is_empty()) {
                db.api_key = legacy.api_key;
            }
        }
        eprintln!("[CONFIG] Merged legacy 'hubcap' entry into 'hubcapdb'");
    } else if let Some(legacy_idx) = hubcap_legacy_idx {
        // Legacy "hubcap" exists but no "hubcapdb" — rename it
        let mut entry = providers.remove(legacy_idx);
        entry.id = "hubcapdb".into();
        if entry.name == "Hubcap" || entry.name.is_empty() {
            entry.name = "HubcapDB".into();
        }
        providers.push(entry);
        eprintln!("[CONFIG] Renamed legacy 'hubcap' to 'hubcapdb'");
    }

    // Remove duplicate IDs (keep first occurrence)
    let mut seen = std::collections::HashSet::new();
    let before_dedup = providers.len();
    providers.retain(|p| seen.insert(p.id.clone()));
    let removed = before_dedup - providers.len();
    if removed > 0 {
        eprintln!("[CONFIG] Removed {removed} duplicate provider(s)");
    }

    // Reorder to canonical order, append unknown IDs at end
    let original_ids: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
    let mut ordered: Vec<ProviderConfig> = Vec::with_capacity(providers.len());
    let mut remaining: Vec<ProviderConfig> = providers.drain(..).collect();

    for &canonical_id in CANONICAL_PROVIDER_IDS {
        if let Some(pos) = remaining.iter().position(|p| p.id == canonical_id) {
            ordered.push(remaining.remove(pos));
        }
    }
    // Append any remaining unknown IDs
    ordered.extend(remaining);

    let changed = ordered.len() != original_len
        || original_ids
            .iter()
            .zip(ordered.iter())
            .any(|(a, b)| a != &b.id);

    *providers = ordered;
    changed
}

/// Ensure the provider list contains all canonical entries with defaults for missing ones.
/// Called during save to guarantee completeness.
fn ensure_canonical_providers(providers: &mut Vec<ProviderConfig>) {
    let defaults = default_providers();
    for default in &defaults {
        if !providers.iter().any(|p| p.id == default.id) {
            eprintln!("[CONFIG] Adding missing default provider: {}", default.id);
            providers.push(default.clone());
        }
    }
}

// ---------------------------------------------------------------------------
// Plugins directory resolution
// ---------------------------------------------------------------------------

/// Canonical runtime plugins directory under `{local_data_dir}/LumaForge/plugins`.
/// The `LUMA_LITE_PLUGINS_DIR` env var override is kept for tests/dev.
pub fn resolve_plugins_dir() -> PathBuf {
    if let Ok(env_dir) = std::env::var("LUMA_LITE_PLUGINS_DIR") {
        let path = PathBuf::from(env_dir);

        if let Err(error) = fs::create_dir_all(&path) {
            eprintln!(
                "[PLUGIN_STORAGE] Failed to create environment plugin directory {}: {error}",
                path.display()
            );
        } else {
            eprintln!(
                "[PLUGIN_STORAGE] Runtime plugins directory from environment: {}",
                path.display()
            );
            return path;
        }
    }

    let plugins_dir = app_data_dir().join("plugins");

    if let Err(error) = fs::create_dir_all(&plugins_dir) {
        eprintln!(
            "[PLUGIN_STORAGE] Failed to create runtime plugins directory {}: {error}",
            plugins_dir.display()
        );
    }

    eprintln!(
        "[PLUGIN_STORAGE] Runtime plugins directory: {}",
        plugins_dir.display()
    );

    plugins_dir
}

/// Resolve the absolute path to a specific plugin's directory.
/// Only looks in the canonical AppData runtime plugins directory.
pub fn resolve_plugin_dir(plugin_id: &str) -> Option<PathBuf> {
    if plugin_id.is_empty()
        || plugin_id == "."
        || plugin_id == ".."
        || plugin_id.contains('/')
        || plugin_id.contains('\\')
    {
        eprintln!("[CONFIG] Invalid plugin ID: {plugin_id}");
        return None;
    }

    let plugin_dir = resolve_plugins_dir().join(plugin_id);

    if plugin_dir.is_dir() {
        Some(plugin_dir)
    } else {
        eprintln!(
            "[CONFIG] Runtime plugin directory not found: {}",
            plugin_dir.display()
        );
        None
    }
}

/// Resolve the absolute path to a specific plugin's inject script.
/// Validates the script path is safe (no traversal) and resides inside the plugin directory.
pub fn resolve_inject_script(plugin_id: &str, script_filename: &str) -> Option<PathBuf> {
    let relative_script = std::path::Path::new(script_filename);

    if script_filename.is_empty()
        || relative_script.is_absolute()
        || relative_script.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        eprintln!(
            "[CONFIG] Invalid inject script path for '{}': {}",
            plugin_id, script_filename
        );
        return None;
    }

    let plugin_dir = resolve_plugin_dir(plugin_id)?;

    let canonical_plugin_dir = match plugin_dir.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "[CONFIG] Failed to canonicalize plugin directory {}: {error}",
                plugin_dir.display()
            );
            return None;
        }
    };

    let script_path = plugin_dir.join(relative_script);

    if !script_path.is_file() {
        eprintln!(
            "[CONFIG] Inject script not found: {}",
            script_path.display()
        );
        return None;
    }

    let canonical_script_path = match script_path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "[CONFIG] Failed to canonicalize inject script {}: {error}",
                script_path.display()
            );
            return None;
        }
    };

    if !canonical_script_path.starts_with(&canonical_plugin_dir) {
        eprintln!(
            "[CONFIG] Inject script escapes plugin directory: {}",
            canonical_script_path.display()
        );
        return None;
    }

    Some(canonical_script_path)
}

// ---------------------------------------------------------------------------
// Steam root detection
// ---------------------------------------------------------------------------

/// Try to detect the Steam installation root directory.
///
/// Priority:
/// 1. `STEAM_PATH` environment variable (if set and directory exists)
/// 2. Windows Registry keys (Valve\Steam)
/// 3. Well-known installation paths
/// 4. `None` if nothing found
pub fn detect_steam_root() -> Option<PathBuf> {
    // 1. Env var override
    if let Ok(env_path) = std::env::var("STEAM_PATH") {
        let p = PathBuf::from(&env_path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Windows Registry
    #[cfg(target_os = "windows")]
    {
        if let Some(p) = detect_steam_from_registry() {
            return Some(p);
        }
    }

    // 3. Well-known paths
    let candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
        PathBuf::from("/Users/Shared/Library/Application Support/Steam"),
        dirs::home_dir()
            .map(|h| h.join(".steam/steam"))
            .unwrap_or_default(),
        dirs::home_dir()
            .map(|h| h.join(".local/share/Steam"))
            .unwrap_or_default(),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.clone());
        }
    }

    None
}

/// Windows Registry lookup for the Steam install path.
#[cfg(target_os = "windows")]
fn detect_steam_from_registry() -> Option<PathBuf> {
    use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);

    let subkeys = [r"SOFTWARE\WOW6432Node\Valve\Steam", r"SOFTWARE\Valve\Steam"];

    for subkey in &subkeys {
        if let Ok(key) = hklm.open_subkey_with_flags(subkey, KEY_READ) {
            if let Ok(value) = key.get_value::<String, _>("InstallPath") {
                eprintln!("[CONFIG] Steam detected via Registry ({subkey}): {value}");
                let path = PathBuf::from(value);
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }

    None
}

/// Return the effective Steam root path (internal helper).
///
/// If the user has configured a custom path, return that.
/// Otherwise, fall back to auto-detection.
pub fn resolve_steam_root() -> Option<PathBuf> {
    let config = load_config();

    if let Some(ref custom) = config.steam_root {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
        eprintln!(
            "[CONFIG] Configured Steam path does not exist: {} — falling back to detection",
            p.display()
        );
    }

    detect_steam_root()
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Return the current Steam root info to the frontend.
#[tauri::command]
pub fn get_steam_root() -> Result<SteamRootInfo, String> {
    let config = load_config();
    let config_path_str = config_path().to_string_lossy().to_string();

    // Check if user has a custom path configured
    if let Some(ref custom) = config.steam_root {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Ok(SteamRootInfo {
                resolved_path: Some(custom.clone()),
                is_custom: true,
                config_path: config_path_str,
            });
        }
    }

    // Fall back to auto-detection
    let detected = detect_steam_root();
    Ok(SteamRootInfo {
        resolved_path: detected.map(|p| p.to_string_lossy().to_string()),
        is_custom: false,
        config_path: config_path_str,
    })
}

/// Set (or clear) the custom Steam root path and persist it.
#[tauri::command]
pub fn set_steam_root(path: Option<String>) -> Result<SteamRootInfo, String> {
    let mut config = load_config().clone();
    config.steam_root = path;
    save_config(&config)?;

    // Return the updated info
    get_steam_root()
}

/// Return the current appearance settings to the frontend.
#[tauri::command]
pub fn get_appearance_settings() -> Result<AppearanceSettings, String> {
    let config = load_config();
    Ok(config.appearance.clone())
}

/// Update appearance settings and persist them.
#[tauri::command]
pub fn set_appearance_settings(settings: AppearanceSettings) -> Result<AppearanceSettings, String> {
    let mut config = load_config().clone();
    config.appearance = settings;
    save_config(&config)?;
    Ok(config.appearance.clone())
}

/// Return the current provider configurations (API keys masked).
#[tauri::command]
pub fn get_providers() -> Result<Vec<ProviderConfigPublic>, String> {
    let config = load_config();
    Ok(config
        .downloads
        .providers
        .iter()
        .map(|p| p.to_public())
        .collect())
}

/// Return the full downloads configuration (API keys masked).
#[tauri::command]
pub fn get_downloads_config() -> Result<DownloadsConfigPublic, String> {
    let config = load_config();
    Ok(DownloadsConfigPublic {
        multi_provider_fallback: config.downloads.multi_provider_fallback,
        providers: config
            .downloads
            .providers
            .iter()
            .map(|p| p.to_public())
            .collect(),
    })
}

/// Public downloads config representation (API keys masked).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadsConfigPublic {
    pub multi_provider_fallback: bool,
    pub providers: Vec<ProviderConfigPublic>,
}

/// Update provider configurations and persist them.
/// The frontend sends the full array; we merge api_key values from the existing
/// config so that masked keys are not saved as literal "••••".
#[tauri::command]
pub fn set_providers(
    mut providers: Vec<ProviderConfig>,
) -> Result<Vec<ProviderConfigPublic>, String> {
    let mut config = load_config().clone();

    // Run migration on incoming providers (normalizes legacy IDs, deduplicates)
    migrate_providers(&mut providers);

    // For each incoming provider, if the api_key looks like a mask or is empty,
    // keep the existing key. Only overwrite when a real new key is provided.
    for incoming in &mut providers {
        if let Some(existing) = config
            .downloads
            .providers
            .iter()
            .find(|p| p.id == incoming.id)
        {
            let key_is_masked = incoming
                .api_key
                .as_deref()
                .map(|k| k.contains('•') || k == "••••" || k.contains("****"))
                .unwrap_or(false);
            let key_is_empty = incoming
                .api_key
                .as_deref()
                .map(|k| k.trim().is_empty())
                .unwrap_or(true);
            if key_is_masked || key_is_empty {
                incoming.api_key = existing.api_key.clone();
            }
        }
    }

    config.downloads.providers = providers;
    ensure_canonical_providers(&mut config.downloads.providers);
    save_config(&config)?;
    Ok(config
        .downloads
        .providers
        .iter()
        .map(|p| p.to_public())
        .collect())
}

/// Return the multi-provider fallback toggle state.
#[tauri::command]
pub fn get_multi_provider_fallback() -> Result<bool, String> {
    let config = load_config();
    Ok(config.downloads.multi_provider_fallback)
}

/// Set the multi-provider fallback toggle and persist it.
#[tauri::command]
pub fn set_multi_provider_fallback(enabled: bool) -> Result<bool, String> {
    let mut config = load_config().clone();
    config.downloads.multi_provider_fallback = enabled;
    save_config(&config)?;
    Ok(config.downloads.multi_provider_fallback)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_providers_has_five_entries() {
        let providers = default_providers();
        assert_eq!(providers.len(), 5);
        assert_eq!(providers[0].id, "hubcapdb");
        assert_eq!(providers[1].id, "ryuu");
        assert_eq!(providers[2].id, "twentytwo");
        assert_eq!(providers[3].id, "sushi");
        assert_eq!(providers[4].id, "custom");
    }

    #[test]
    fn migration_fresh_providers_unchanged() {
        let mut providers = default_providers();
        assert!(!migrate_providers(&mut providers));
        assert_eq!(providers.len(), 5);
        assert_eq!(providers[0].id, "hubcapdb");
    }

    #[test]
    fn migration_merges_legacy_hubcap_into_hubcapdb() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: Some(String::new()),
            },
            ProviderConfig {
                id: "hubcap".into(),
                name: "Hubcap".into(),
                enabled: true,
                base_url: "https://old-hubcap.example.com".into(),
                api_key: Some("real-key-123".into()),
            },
        ];
        let changed = migrate_providers(&mut providers);
        assert!(changed);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "hubcapdb");
        // Legacy key should be preserved since hubcapdb had empty key
        assert_eq!(providers[0].api_key.as_deref(), Some("real-key-123"));
        // hubcapdb's base_url is NOT the placeholder, so it is preserved
        assert_eq!(providers[0].base_url, "https://hubcapmanifest.com");
    }

    #[test]
    fn migration_prefers_hubcapdb_values_over_legacy() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: Some("db-key".into()),
            },
            ProviderConfig {
                id: "hubcap".into(),
                name: "Hubcap".into(),
                enabled: true,
                base_url: "https://old-hubcap.example.com".into(),
                api_key: Some("legacy-key".into()),
            },
        ];
        migrate_providers(&mut providers);
        // hubcapdb already has a real key, so legacy key should NOT overwrite
        assert_eq!(providers[0].api_key.as_deref(), Some("db-key"));
    }

    #[test]
    fn migration_uses_legacy_values_when_hubcapdb_has_placeholder() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "hubcap".into(),
                name: "Hubcap".into(),
                enabled: true,
                base_url: "https://real-hubcap.com".into(),
                api_key: Some("real-key".into()),
            },
        ];
        migrate_providers(&mut providers);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "hubcapdb");
        // hubcapdb had placeholder URL, so legacy URL is used
        assert_eq!(providers[0].base_url, "https://real-hubcap.com");
        // hubcapdb had no key, so legacy key is used
        assert_eq!(providers[0].api_key.as_deref(), Some("real-key"));
    }

    #[test]
    fn migration_renames_legacy_hubcap_when_no_hubcapdb() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcap".into(),
                name: "Hubcap".into(),
                enabled: true,
                base_url: "https://old-hubcap.example.com".into(),
                api_key: Some("my-key".into()),
            },
            ProviderConfig {
                id: "ryuu".into(),
                name: "Ryuu".into(),
                enabled: false,
                base_url: "https://generator.ryuu.lol".into(),
                api_key: None,
            },
        ];
        let changed = migrate_providers(&mut providers);
        assert!(changed);
        assert_eq!(providers.len(), 2);
        // Should be renamed to hubcapdb
        assert_eq!(providers[0].id, "hubcapdb");
        assert_eq!(providers[0].name, "HubcapDB");
        assert_eq!(providers[0].api_key.as_deref(), Some("my-key"));
    }

    #[test]
    fn migration_deduplicates_by_id() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: Some("key1".into()),
            },
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB Duplicate".into(),
                enabled: false,
                base_url: "https://other.com".into(),
                api_key: Some("key2".into()),
            },
        ];
        let changed = migrate_providers(&mut providers);
        assert!(changed);
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id, "hubcapdb");
    }

    #[test]
    fn migration_reorders_to_canonical_order() {
        let mut providers = vec![
            ProviderConfig {
                id: "custom".into(),
                name: "Custom".into(),
                enabled: false,
                base_url: "https://api.example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "sushi".into(),
                name: "Sushi".into(),
                enabled: false,
                base_url: "https://sushi.example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: None,
            },
        ];
        let changed = migrate_providers(&mut providers);
        assert!(changed);
        assert_eq!(providers.len(), 3);
        assert_eq!(providers[0].id, "hubcapdb");
        assert_eq!(providers[1].id, "sushi");
        assert_eq!(providers[2].id, "custom");
    }

    #[test]
    fn migration_preserves_unknown_ids_at_end() {
        let mut providers = vec![
            ProviderConfig {
                id: "custom".into(),
                name: "Custom".into(),
                enabled: false,
                base_url: "https://api.example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "unknown-provider".into(),
                name: "Unknown".into(),
                enabled: true,
                base_url: "https://unknown.example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: None,
            },
        ];
        migrate_providers(&mut providers);
        // hubcapdb first, then custom, then unknown-provider (appended at end)
        assert_eq!(providers[0].id, "hubcapdb");
        assert_eq!(providers[1].id, "custom");
        assert_eq!(providers[2].id, "unknown-provider");
    }

    #[test]
    fn migration_idempotent() {
        let mut providers = vec![
            ProviderConfig {
                id: "hubcapdb".into(),
                name: "HubcapDB".into(),
                enabled: true,
                base_url: "https://hubcapmanifest.com".into(),
                api_key: Some("key".into()),
            },
            ProviderConfig {
                id: "ryuu".into(),
                name: "Ryuu".into(),
                enabled: false,
                base_url: "https://generator.ryuu.lol".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "twentytwo".into(),
                name: "TwentyTwo Cloud".into(),
                enabled: false,
                base_url: "https://api.twentytwocloud.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "sushi".into(),
                name: "Sushi".into(),
                enabled: false,
                base_url: "https://sushi.example.com".into(),
                api_key: None,
            },
            ProviderConfig {
                id: "custom".into(),
                name: "Custom API".into(),
                enabled: false,
                base_url: "https://api.example.com".into(),
                api_key: None,
            },
        ];
        let first_run = migrate_providers(&mut providers);
        let ids_after_first: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        let second_run = migrate_providers(&mut providers);
        let ids_after_second: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        // First run on already-canonical list should return false
        assert!(!first_run);
        // Second run also returns false
        assert!(!second_run);
        assert_eq!(ids_after_first, ids_after_second);
    }

    #[test]
    fn ensure_canonical_adds_missing_defaults() {
        let mut providers = vec![ProviderConfig {
            id: "hubcapdb".into(),
            name: "HubcapDB".into(),
            enabled: true,
            base_url: "https://hubcapmanifest.com".into(),
            api_key: None,
        }];
        ensure_canonical_providers(&mut providers);
        assert_eq!(providers.len(), 5);
        let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"hubcapdb"));
        assert!(ids.contains(&"ryuu"));
        assert!(ids.contains(&"twentytwo"));
        assert!(ids.contains(&"sushi"));
        assert!(ids.contains(&"custom"));
    }

    #[test]
    fn provider_to_public_masks_api_key() {
        let provider = ProviderConfig {
            id: "hubcapdb".into(),
            name: "HubcapDB".into(),
            enabled: true,
            base_url: "https://hubcapmanifest.com".into(),
            api_key: Some("sk-abcdef1234567890xyz".into()),
        };
        let public = provider.to_public();
        assert!(public.has_api_key);
        assert!(public.key_preview.starts_with("sk-a"));
        assert!(public.key_preview.ends_with("xyz"));
        assert!(!public.key_preview.contains("abcdef1234567890xyz"));
        assert!(public.adapter_available);
    }

    #[test]
    fn provider_to_public_no_key() {
        let provider = ProviderConfig {
            id: "ryuu".into(),
            name: "Ryuu".into(),
            enabled: false,
            base_url: "https://generator.ryuu.lol".into(),
            api_key: None,
        };
        let public = provider.to_public();
        assert!(!public.has_api_key);
        assert!(public.key_preview.is_empty());
        assert!(!public.adapter_available);
    }

    #[test]
    fn full_json_roundtrip() {
        let config = AppConfig {
            steam_root: Some(r"C:\Steam".into()),
            appearance: AppearanceSettings::default(),
            downloads: DownloadsConfig {
                multi_provider_fallback: true,
                providers: default_providers(),
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.steam_root, config.steam_root);
        assert_eq!(restored.downloads.providers.len(), 5);
        assert_eq!(restored.downloads.providers[0].id, "hubcapdb");
    }
}
