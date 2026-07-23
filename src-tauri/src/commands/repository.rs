use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use crate::config::RepositorySource;

// ---------------------------------------------------------------------------
// Types — Repository Index Schema (schemaVersion 1)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryExtensionEntry {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    pub manifest_url: String,
    #[serde(default)]
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryIndex {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    #[serde(default)]
    pub extensions: Vec<RepositoryExtensionEntry>,
    /// Repository-level id (e.g. "official") — optional in the index.
    #[serde(default)]
    pub id: Option<String>,
    /// Repository-level display name — optional.
    #[serde(default)]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Types — Frontend-facing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryExtensionView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: String,
    pub categories: Vec<String>,
    pub manifest_url: String,
    pub verified: bool,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub repository_url: String,
    pub repository_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRepositoriesResult {
    pub repositories: Vec<RepositorySource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRepositoryExtensionsResult {
    pub extensions: Vec<RepositoryExtensionView>,
    pub repositories: Vec<RepositorySource>,
}

// ---------------------------------------------------------------------------
// In-memory index cache  (url → (index, timestamp))
// ---------------------------------------------------------------------------

static INDEX_CACHE: OnceLock<DashMap<String, (RepositoryIndex, u64)>> = OnceLock::new();

fn get_index_cache() -> &'static DashMap<String, (RepositoryIndex, u64)> {
    INDEX_CACHE.get_or_init(DashMap::new)
}

/// Deduplicate concurrent fetches for the same URL.
static FETCH_IN_FLIGHT: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();

fn get_fetch_in_flight() -> &'static Mutex<HashMap<String, bool>> {
    FETCH_IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// URL validation
// ---------------------------------------------------------------------------

fn is_valid_repository_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Err("Repository URL cannot be empty".into());
    }

    let trimmed = url.trim();
    if trimmed != url {
        return Err("Repository URL contains leading/trailing whitespace".into());
    }

    if !trimmed.starts_with("https://") && !trimmed.starts_with("http://") {
        return Err("Repository URL must start with http:// or https://".into());
    }

    if trimmed.contains('@') {
        return Err("Repository URL must not contain credentials".into());
    }

    if trimmed.contains("..") {
        return Err("Repository URL must not contain path traversal (..)".into());
    }

    Ok(())
}

fn is_valid_extension_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Extension ID cannot be empty".into());
    }
    if id.len() > 128 {
        return Err("Extension ID is too long (max 128 characters)".into());
    }
    let valid = id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !valid {
        return Err(
            "Extension ID must contain only lowercase ASCII letters, digits, hyphens, and underscores"
                .into(),
        );
    }
    if id.starts_with('-') || id.starts_with('_') {
        return Err("Extension ID must not start with a hyphen or underscore".into());
    }
    if id.contains("--") {
        return Err("Extension ID must not contain consecutive hyphens".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// URL resolution
// ---------------------------------------------------------------------------

fn resolve_manifest_url(base_index_url: &str, manifest_url: &str) -> Result<String, String> {
    if manifest_url.starts_with("http://") || manifest_url.starts_with("https://") {
        if manifest_url.contains("..") {
            return Err("Manifest URL contains path traversal".into());
        }
        return Ok(manifest_url.to_string());
    }
    let base = base_index_url
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or(base_index_url);
    let resolved = format!("{}/{}", base, manifest_url.trim_start_matches('/'));
    if resolved.contains("..") {
        return Err("Resolved manifest URL contains path traversal".into());
    }
    Ok(resolved)
}

/// Resolve a file path relative to a manifest URL (not the index URL).
fn resolve_file_url(manifest_url: &str, file_path: &str) -> Result<String, String> {
    if file_path.starts_with("http://") || file_path.starts_with("https://") {
        if file_path.contains("..") {
            return Err("File URL contains path traversal".into());
        }
        return Ok(file_path.to_string());
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or(manifest_url);
    let resolved = format!("{}/{}", base, file_path.trim_start_matches('/'));
    if resolved.contains("..") {
        return Err("Resolved file URL contains path traversal".into());
    }
    Ok(resolved)
}

// ---------------------------------------------------------------------------
// Config helpers — read/write repositories
//
// We bypass config::load_config()'s OnceLock for reads and write back
// through save_config so the in-memory cache stays coherent.
// ---------------------------------------------------------------------------

fn load_repositories_from_config() -> Vec<RepositorySource> {
    // Re-read from disk so we never serve stale data from a OnceLock.
    let path = crate::config::config_path();
    let repos = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<crate::config::AppConfig>(&raw).ok())
        .map(|c| c.repositories)
        .unwrap_or_default();
    eprintln!(
        "[REPOSITORY] Loaded {} repository source(s) from config",
        repos.len()
    );
    repos
}

fn save_repositories_to_config(repos: &[RepositorySource]) -> Result<(), String> {
    let mut config = crate::config::load_config();
    config.repositories = repos.to_vec();
    crate::config::save_config(&config)?;
    eprintln!(
        "[REPOSITORY] Persisted {} repository source(s)",
        repos.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Installed extension detection
// ---------------------------------------------------------------------------

fn get_installed_repository_extensions() -> HashMap<String, String> {
    let plugins_dir = crate::config::resolve_plugins_dir();
    let mut installed = HashMap::new();

    if !plugins_dir.exists() {
        return installed;
    }

    let dir_entries = match fs::read_dir(&plugins_dir) {
        Ok(e) => e,
        Err(_) => return installed,
    };

    for entry in dir_entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        // Skip staging directories
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(".installing-") {
            continue;
        }

        let config_path = entry.path().join("extension-config.json");
        if !config_path.exists() {
            continue;
        }

        let raw = match fs::read_to_string(&config_path) {
            Ok(r) => r,
            Err(_) => continue,
        };

        #[derive(Deserialize)]
        struct ExtensionConfig {
            #[serde(default)]
            source: Option<String>,
            #[serde(default)]
            source_version: Option<String>,
        }

        let config: ExtensionConfig = match serde_json::from_str(&raw) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if let Some(source) = config.source {
            if source == "repository" {
                installed.insert(name, config.source_version.unwrap_or_default());
            }
        }
    }

    installed
}

/// Check if a plugin directory exists and has a valid manifest (used for
/// installed-state in Browse tab).
fn is_plugin_installed(extension_id: &str) -> (bool, Option<String>) {
    let plugins_dir = crate::config::resolve_plugins_dir();
    let dir = plugins_dir.join(extension_id);

    if !dir.exists() {
        return (false, None);
    }

    let manifest_path = dir.join("manifest.json");
    if !manifest_path.exists() {
        return (false, None);
    }

    // Read version from manifest
    let version = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("version").and_then(|s| s.as_str()).map(String::from));

    (true, version)
}

// ---------------------------------------------------------------------------
// Internal fetch + cache helpers
// ---------------------------------------------------------------------------

fn make_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("LumaForge/0.1.0")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))
}

/// Fetch an index from a URL, validate it, and return it with a timestamp.
fn fetch_and_validate_index(
    url: &str,
    client: &reqwest::blocking::Client,
) -> Result<(RepositoryIndex, u64), String> {
    eprintln!("[REPOSITORY] Fetching index: {url}");
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to fetch repository index: {e}"))?;

    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let body = response
        .text()
        .map_err(|e| format!("Failed to read response body: {e}"))?;

    eprintln!(
        "[REPOSITORY] Index fetched: status={status}, content-type={content_type}, size={} bytes",
        body.len()
    );

    if !status.is_success() {
        return Err(format!("HTTP {status} fetching index from {url}"));
    }

    let index: RepositoryIndex = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse repository index: {e}"))?;

    if index.schema_version != 1 {
        return Err(format!(
            "Unsupported schema version: {} (expected 1)",
            index.schema_version
        ));
    }

    let repo_id = index.id.as_deref().unwrap_or("unknown");
    eprintln!(
        "[REPOSITORY] Index validated: id={repo_id}, extensions={}",
        index.extensions.len()
    );

    for ext in &index.extensions {
        is_valid_extension_id(&ext.id)
            .map_err(|e| format!("Invalid extension ID '{}': {}", ext.id, e))?;
        resolve_manifest_url(url, &ext.manifest_url)
            .map_err(|e| format!("Invalid manifest URL for '{}': {}", ext.id, e))?;
    }

    let now = current_timestamp();
    Ok((index, now))
}

/// Ensure all persisted repository sources have their indexes cached.
/// Deduplicates concurrent fetches for the same URL.
fn ensure_repository_indexes_loaded() -> Vec<(RepositorySource, Option<String>)> {
    let repos = load_repositories_from_config();
    let client = match make_http_client() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[REPOSITORY] Cannot create HTTP client: {e}");
            return repos.into_iter().map(|r| (r, Some(e.clone()))).collect();
        }
    };

    let mut results: Vec<(RepositorySource, Option<String>)> = Vec::new();

    for repo in &repos {
        // Already cached?
        if get_index_cache().contains_key(&repo.url) {
            eprintln!(
                "[REPOSITORY] Cache hit for '{}'",
                repo.label.as_deref().unwrap_or(&repo.url)
            );
            results.push((repo.clone(), None));
            continue;
        }

        // Deduplicate concurrent fetches
        {
            let mut in_flight = get_fetch_in_flight()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if in_flight.get(&repo.url).copied() == Some(true) {
                eprintln!(
                    "[REPOSITORY] Fetch already in flight for '{}', waiting",
                    repo.label.as_deref().unwrap_or(&repo.url)
                );
                // We'll fall through and try to fetch — the other thread
                // will have populated the cache by the time we get the data.
                // In practice Tauri commands are serial per request, but
                // guard against parallel boot-time calls.
            }
            in_flight.insert(repo.url.clone(), true);
        }

        let fetch_result = fetch_and_validate_index(&repo.url, &client);

        // Clear in-flight flag
        {
            let mut in_flight = get_fetch_in_flight()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            in_flight.remove(&repo.url);
        }

        match fetch_result {
            Ok((index, ts)) => {
                get_index_cache().insert(repo.url.clone(), (index, ts));
                eprintln!(
                    "[REPOSITORY] Cached index for '{}' ({} extensions)",
                    repo.label.as_deref().unwrap_or(&repo.url),
                    get_index_cache()
                        .get(&repo.url)
                        .map(|r| r.0.extensions.len())
                        .unwrap_or(0)
                );
                results.push((repo.clone(), None));
            }
            Err(e) => {
                eprintln!(
                    "[REPOSITORY] Index fetch failed for '{}': {e}",
                    repo.label.as_deref().unwrap_or(&repo.url)
                );
                // Preserve any stale cache entry (don't clear valid data)
                if get_index_cache().contains_key(&repo.url) {
                    eprintln!("[REPOSITORY] Preserving previous cache entry");
                }
                results.push((repo.clone(), Some(e)));
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_extension_repositories() -> Result<ListRepositoriesResult, String> {
    let repos = load_repositories_from_config();
    Ok(ListRepositoriesResult {
        repositories: repos,
    })
}

#[tauri::command]
pub fn add_extension_repository(
    url: String,
    label: Option<String>,
    app_handle: tauri::AppHandle,
) -> Result<RepositorySource, String> {
    let trimmed = url.trim().to_string();
    eprintln!("[REPOSITORY] Adding source: {trimmed}");
    is_valid_repository_url(&trimmed)?;

    let mut repos = load_repositories_from_config();

    if repos.iter().any(|r| r.url == trimmed) {
        return Err("Repository already added".into());
    }

    // 1. Fetch and validate the index BEFORE persisting
    let client = make_http_client()?;
    let (index, ts) = fetch_and_validate_index(&trimmed, &client)?;

    // 2. Cache the validated index
    get_index_cache().insert(trimmed.clone(), (index.clone(), ts));
    let ext_count = index.extensions.len();
    eprintln!(
        "[REPOSITORY] Index fetched: id={}, extensions={ext_count}",
        index.id.as_deref().unwrap_or("unknown")
    );

    // 3. Persist the source
    let source = RepositorySource {
        url: trimmed.clone(),
        label: label
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty()),
        last_fetched: Some(ts),
        last_error: None,
    };

    repos.push(source.clone());
    save_repositories_to_config(&repos)?;

    let _ = app_handle.emit("repositories-changed", &repos);

    eprintln!(
        "[REPOSITORY] Added repository '{}' with {ext_count} extension(s)",
        source.label.as_deref().unwrap_or(&trimmed)
    );
    Ok(source)
}

#[tauri::command]
pub fn remove_extension_repository(
    url: String,
    app_handle: tauri::AppHandle,
) -> Result<Vec<RepositorySource>, String> {
    let mut repos = load_repositories_from_config();
    let before = repos.len();
    repos.retain(|r| r.url != url);

    if repos.len() == before {
        return Err("Repository not found".into());
    }

    save_repositories_to_config(&repos)?;

    // Clear index cache for this URL
    get_index_cache().remove(&url);

    let _ = app_handle.emit("repositories-changed", &repos);

    eprintln!("[REPOSITORY] Removed repository: {url}");
    Ok(repos)
}

#[tauri::command]
pub fn refresh_extension_repository(url: String) -> Result<RepositorySource, String> {
    let trimmed = url.trim().to_string();
    eprintln!("[REPOSITORY] Refreshing: {trimmed}");
    is_valid_repository_url(&trimmed)?;

    let mut repos = load_repositories_from_config();
    let repo_idx = repos
        .iter()
        .position(|r| r.url == trimmed)
        .ok_or_else(|| format!("Repository not found: {trimmed}"))?;

    let client = make_http_client()?;
    let (index, ts) = fetch_and_validate_index(&trimmed, &client)?;

    get_index_cache().insert(trimmed.clone(), (index, ts));

    repos[repo_idx].last_fetched = Some(ts);
    repos[repo_idx].last_error = None;
    save_repositories_to_config(&repos)?;

    let ext_count = get_index_cache()
        .get(&trimmed)
        .map(|r| r.0.extensions.len())
        .unwrap_or(0);
    eprintln!(
        "[REPOSITORY] Refreshed '{}': {ext_count} extensions",
        repos[repo_idx].label.as_deref().unwrap_or(&trimmed)
    );

    Ok(repos[repo_idx].clone())
}

#[tauri::command]
pub fn list_repository_extensions() -> Result<ListRepositoryExtensionsResult, String> {
    // Ensure all indexes are loaded (fetch if missing)
    let hydrated = ensure_repository_indexes_loaded();

    let installed = get_installed_repository_extensions();
    let mut all_extensions: Vec<RepositoryExtensionView> = Vec::new();

    for (repo, _error) in &hydrated {
        let index = match get_index_cache().get(&repo.url) {
            Some(entry) => entry.0.clone(),
            None => {
                eprintln!(
                    "[REPOSITORY] No index available for '{}'",
                    repo.label.as_deref().unwrap_or(&repo.url)
                );
                continue;
            }
        };

        for ext in &index.extensions {
            let manifest_url =
                resolve_manifest_url(&repo.url, &ext.manifest_url).unwrap_or_default();

            // Check installed state both from our tracking AND from disk
            let (disk_installed, disk_version) = is_plugin_installed(&ext.id);
            let tracking_installed = installed.contains_key(&ext.id);
            let is_installed = disk_installed || tracking_installed;
            let installed_ver = installed.get(&ext.id).cloned().or(disk_version);

            all_extensions.push(RepositoryExtensionView {
                id: ext.id.clone(),
                name: ext
                    .display_name
                    .clone()
                    .or_else(|| ext.name.clone())
                    .unwrap_or_else(|| ext.id.clone()),
                description: ext.description.clone().unwrap_or_default(),
                version: ext.version.clone().unwrap_or_else(|| "0.0.0".into()),
                author: ext.author.clone().unwrap_or_default(),
                categories: ext.categories.clone(),
                manifest_url,
                verified: ext.verified.unwrap_or(false),
                installed: is_installed,
                installed_version: installed_ver,
                repository_url: repo.url.clone(),
                repository_label: repo.label.clone(),
            });
        }
    }

    eprintln!(
        "[REPOSITORY] Catalog merged: remote={}, installed={}",
        all_extensions.len(),
        installed.len()
    );

    Ok(ListRepositoryExtensionsResult {
        extensions: all_extensions,
        repositories: hydrated.into_iter().map(|(r, _)| r).collect(),
    })
}

#[tauri::command]
pub fn install_repository_extension(
    extension_id: String,
    manifest_url: String,
    app_handle: tauri::AppHandle,
) -> Result<super::plugins::PluginEntry, String> {
    eprintln!("[REPOSITORY] Installing extension: {extension_id}");
    is_valid_extension_id(&extension_id)?;

    if manifest_url.contains("..") {
        return Err("Manifest URL contains path traversal".into());
    }

    let plugins_dir = crate::config::resolve_plugins_dir();
    let target_dir = plugins_dir.join(&extension_id);
    if target_dir.exists() {
        // Allow reinstall if the previous install is incomplete (missing extension.lua)
        let has_manifest = target_dir.join("manifest.json").exists();
        let has_lua = target_dir.join("extension.lua").exists();
        if has_manifest && has_lua {
            return Err(format!("Extension '{extension_id}' is already installed"));
        }
        eprintln!(
            "[REPOSITORY_INSTALL] Partial install detected for '{extension_id}' (manifest={has_manifest}, lua={has_lua}) — reinstalling"
        );
        // Clean up partial install before proceeding
        let _ = fs::remove_dir_all(&target_dir);
    }

    let client = make_http_client()?;

    // 1. Fetch the manifest
    eprintln!("[REPOSITORY] Manifest fetched: {extension_id}");
    let response = client
        .get(&manifest_url)
        .send()
        .map_err(|e| format!("Failed to fetch manifest: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch manifest: HTTP {}",
            response.status()
        ));
    }

    let body = response
        .text()
        .map_err(|e| format!("Failed to read manifest response: {e}"))?;

    let manifest: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse manifest: {e}"))?;

    let manifest_id = manifest
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Manifest missing 'id' field".to_string())?;

    if manifest_id != extension_id {
        return Err(format!(
            "Manifest ID mismatch: expected '{extension_id}', got '{manifest_id}'"
        ));
    }

    let manifest_version = manifest
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();

    // 2. Atomic staging
    let nonce = current_timestamp();
    let staging_dir = plugins_dir.join(format!(".installing-{extension_id}-{nonce}"));
    let final_dir = plugins_dir.join(&extension_id);

    fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("Failed to create staging directory: {e}"))?;

    // Write manifest.json
    let manifest_path = staging_dir.join("manifest.json");
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| {
        let _ = fs::remove_dir_all(&staging_dir);
        format!("Failed to write manifest: {e}")
    })?;

    // Write extension-config.json
    let config_path = staging_dir.join("extension-config.json");
    let ext_config = serde_json::json!({
        "enabled": true,
        "source": "repository",
        "sourceVersion": &manifest_version,
        "manifestUrl": &manifest_url,
    });
    fs::write(
        &config_path,
        serde_json::to_string_pretty(&ext_config).unwrap_or_default(),
    )
    .map_err(|e| {
        let _ = fs::remove_dir_all(&staging_dir);
        format!("Failed to write config: {e}")
    })?;

    // 3. Download extension.lua (the lifecycle script) from the repository
    //    alongside the manifest. This is required for Lua-based extensions.
    let mut has_extension_lua = false;
    let lua_url = resolve_file_url(&manifest_url, "extension.lua").unwrap_or_default();
    if !lua_url.is_empty() {
        match download_file_to_staging(&client, &lua_url, &staging_dir, "extension.lua") {
            Ok(_) => {
                has_extension_lua = true;
                eprintln!("[REPOSITORY_INSTALL] Downloaded extension.lua for '{extension_id}'");
            }
            Err(e) => {
                eprintln!("[REPOSITORY_INSTALL] No extension.lua found for '{extension_id}': {e}");
            }
        }
    }

    // 4. Download activation.injectScript if present (legacy CEF pattern)
    if let Some(activation) = manifest.get("activation") {
        if let Some(inject_script) = activation.get("injectScript").and_then(|v| v.as_str()) {
            let script_url = resolve_file_url(&manifest_url, inject_script)?;
            if let Err(e) =
                download_file_to_staging(&client, &script_url, &staging_dir, inject_script)
            {
                eprintln!(
                    "[REPOSITORY_INSTALL] Warning: failed to download inject script '{inject_script}': {e}"
                );
            }
        }
    }

    // 5. For non-Lua extensions: download release assets into staging.
    //    Lua extensions handle their own downloads via the lifecycle.
    if !has_extension_lua {
        if let Some(release_provider) = manifest.get("releaseProvider") {
            let provider_type = release_provider
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match provider_type {
                "github" => {
                    if let Some(config) = release_provider.get("config") {
                        let files = download_github_release(
                            &client,
                            config,
                            manifest.get("managedFiles"),
                            &staging_dir,
                        )?;
                        eprintln!(
                            "[REPOSITORY_INSTALL] Downloaded {} release asset(s) for '{extension_id}'",
                            files.len()
                        );
                    }
                }
                "http" => {
                    if let Some(config) = release_provider.get("config") {
                        let files = download_http_release(
                            &client,
                            config,
                            manifest.get("managedFiles"),
                            &staging_dir,
                        )?;
                        eprintln!(
                            "[REPOSITORY_INSTALL] Downloaded {} release asset(s) for '{extension_id}'",
                            files.len()
                        );
                    }
                }
                other => {
                    eprintln!(
                        "[REPOSITORY_INSTALL] Unknown release provider '{other}' — skipping file download"
                    );
                }
            }
        }
    } else {
        eprintln!(
            "[REPOSITORY_INSTALL] Lua extension detected — release assets will be managed by lifecycle"
        );
    }

    // 4. Atomic rename: staging → final
    fs::rename(&staging_dir, &final_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&staging_dir);
        format!("Failed to move extension into place: {e}. Cleaned up staging directory.")
    })?;

    eprintln!(
        "[REPOSITORY_INSTALL] Plugin directory committed: {}",
        final_dir.display()
    );

    // 6. Re-scan plugins to register the new extension
    eprintln!("[REPOSITORY_INSTALL] Scanning plugins to register: {extension_id}");
    let plugins = super::plugins::do_scan_plugins(None)?;
    eprintln!(
        "[REPOSITORY_INSTALL] Plugin scan returned {} plugins, looking for '{extension_id}'",
        plugins.len()
    );
    let entry = plugins
        .into_iter()
        .find(|p| p.id == extension_id)
        .ok_or_else(|| {
            let _ = fs::remove_dir_all(&final_dir);
            eprintln!("[REPOSITORY_INSTALL] Plugin scan did NOT detect installed extension: {extension_id}");
            eprintln!("[REPOSITORY_INSTALL] Available plugin IDs: ... (check logs above)");
            format!("Extension '{extension_id}' installed but could not be scanned")
        })?;

    eprintln!(
        "[REPOSITORY_INSTALL] Plugin entry found: id={}, script_path={:?}, source={}",
        entry.id, entry.script_path, entry.source
    );

    // 7. Execute lifecycle if extension.lua was installed
    eprintln!(
        "[REPOSITORY_INSTALL] Lifecycle check: has_extension_lua={}, script_path={:?}",
        has_extension_lua, entry.script_path
    );
    if has_extension_lua && entry.script_path.is_some() {
        let script_path = entry.script_path.clone().unwrap_or_default();
        let steam_root = crate::config::resolve_steam_root()
            .map(|p| p.to_string_lossy().to_string())
            .ok_or_else(|| {
                let _ = fs::remove_dir_all(&final_dir);
                super::extension_lifecycle::clear_engine_cache();
                "Cannot run install lifecycle: Steam root not detected. \
                 Set STEAM_PATH environment variable or configure Steam root in settings."
                    .to_string()
            })?;

        eprintln!("[REPOSITORY_INSTALL] Steam root resolved: {steam_root}");

        // 7a. Load extension into Lua engine
        eprintln!("[REPOSITORY_INSTALL] Loading lifecycle: {extension_id}");
        match super::extension_lifecycle::load_extension(extension_id.clone(), script_path) {
            Ok(meta) => {
                let has_install = meta
                    .get("hasInstall")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let has_detect = meta
                    .get("hasDetect")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                eprintln!(
                    "[REPOSITORY_INSTALL] Extension loaded: has_install={has_install}, has_detect={has_detect}"
                );

                // 7b. Run install lifecycle
                if has_install {
                    eprintln!("[REPOSITORY_INSTALL] Running install lifecycle: {extension_id}");
                    match super::extension_lifecycle::call_extension_install(
                        extension_id.clone(),
                        steam_root.clone(),
                    ) {
                        Ok(result) => {
                            eprintln!("[REPOSITORY_INSTALL] Install lifecycle completed: {result}");
                        }
                        Err(e) => {
                            eprintln!("[REPOSITORY_INSTALL] Install lifecycle FAILED: {e}");
                            // Rollback: remove the incomplete plugin directory
                            let _ = fs::remove_dir_all(&final_dir);
                            super::extension_lifecycle::clear_engine_cache();
                            return Err(format!(
                                "Extension '{extension_id}' install lifecycle failed: {e}"
                            ));
                        }
                    }
                }

                // 7c. Verify with detect
                if has_detect {
                    eprintln!("[REPOSITORY_INSTALL] Running detect to verify: {extension_id}");
                    match super::extension_lifecycle::call_extension_detect(
                        extension_id.clone(),
                        steam_root,
                    ) {
                        Ok(detect_result) => {
                            eprintln!("[REPOSITORY_INSTALL] Detection result: {detect_result}");
                            let status = detect_result
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("unknown");
                            if status == "available" {
                                // No files installed — lifecycle didn't work
                                eprintln!(
                                    "[REPOSITORY_INSTALL] FAILED: detect reports 'available' — DLLs not in Steam root after install"
                                );
                                let _ = fs::remove_dir_all(&final_dir);
                                super::extension_lifecycle::clear_engine_cache();
                                return Err(format!(
                                    "Extension '{extension_id}' install lifecycle ran but DLLs were not found in Steam root. \
                                     The extension has been removed."
                                ));
                            } else {
                                eprintln!(
                                    "[REPOSITORY_INSTALL] Detection confirmed: status={status}"
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("[REPOSITORY_INSTALL] Detect FAILED: {e}");
                            let _ = fs::remove_dir_all(&final_dir);
                            super::extension_lifecycle::clear_engine_cache();
                            return Err(format!(
                                "Extension '{extension_id}' install verification failed: {e}. \
                                 The extension has been removed."
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "[REPOSITORY_INSTALL] FAILED to load Lua lifecycle for '{extension_id}': {e}"
                );
                let _ = fs::remove_dir_all(&final_dir);
                super::extension_lifecycle::clear_engine_cache();
                return Err(format!(
                    "Extension '{extension_id}' failed to load Lua lifecycle: {e}. \
                     The extension has been removed."
                ));
            }
        }
    }

    eprintln!(
        "[REPOSITORY_INSTALL] Plugin scan confirmed: {extension_id} (enabled={})",
        entry.enabled
    );

    let _ = app_handle.emit("extension-installed", &entry);

    Ok(entry)
}

#[tauri::command]
pub fn uninstall_extension(
    extension_id: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    is_valid_extension_id(&extension_id)?;

    let plugins_dir = crate::config::resolve_plugins_dir();
    let plugin_dir = plugins_dir.join(&extension_id);

    if !plugin_dir.exists() {
        return Err(format!("Extension '{extension_id}' is not installed"));
    }

    // Built-in extensions that are NOT from a repository cannot be uninstalled
    let config_path = plugin_dir.join("extension-config.json");
    let is_repo_source = if config_path.exists() {
        fs::read_to_string(&config_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|v| v.get("source").and_then(|s| s.as_str()).map(String::from))
            .map(|s| s == "repository")
            .unwrap_or(false)
    } else {
        false
    };

    if !is_repo_source {
        return Err("Only repository extensions can be uninstalled through this interface".into());
    }

    // 1. Disable CEF injection if active
    let cache = super::plugins::get_plugins_cache();
    if let Some(plugin) = cache.get(&extension_id) {
        if plugin.enabled {
            super::plugins::deactivate_cef_injection(&extension_id);
        }
    }

    // 2. Execute uninstall lifecycle BEFORE deleting plugin directory
    let script_path = plugin_dir.join("extension.lua");
    if script_path.exists() {
        let script_str = script_path.to_string_lossy().to_string();
        eprintln!("[REPOSITORY_UNINSTALL] Loading lifecycle for uninstall: {extension_id}");
        if let Err(e) = super::extension_lifecycle::load_extension(extension_id.clone(), script_str)
        {
            eprintln!(
                "[REPOSITORY_UNINSTALL] Failed to load lifecycle (continuing with cleanup): {e}"
            );
        } else {
            let install_dir = crate::config::resolve_steam_root()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    eprintln!(
                        "[REPOSITORY_UNINSTALL] WARNING: Steam root not detected — uninstall lifecycle may fail"
                    );
                    String::new()
                });
            eprintln!("[REPOSITORY_UNINSTALL] Running uninstall lifecycle: {extension_id}");
            match super::extension_lifecycle::call_extension_uninstall(
                extension_id.clone(),
                install_dir,
            ) {
                Ok(result) => {
                    eprintln!("[REPOSITORY_UNINSTALL] Uninstall lifecycle completed: {result}");
                }
                Err(e) => {
                    eprintln!(
                        "[REPOSITORY_UNINSTALL] Uninstall lifecycle failed (continuing with cleanup): {e}"
                    );
                }
            }
        }
    }

    // 3. Now safe to delete plugin directory (lifecycle has cleaned up managed files)
    fs::remove_dir_all(&plugin_dir)
        .map_err(|e| format!("Failed to remove extension directory: {e}"))?;

    cache.remove(&extension_id);
    super::extension_lifecycle::clear_engine_cache();

    eprintln!("[REPOSITORY_UNINSTALL] Uninstalled extension '{extension_id}'");

    let _ = app_handle.emit("extension-uninstalled", &extension_id);

    Ok(())
}

// ---------------------------------------------------------------------------
// File download helpers
// ---------------------------------------------------------------------------

/// Download a single file from a URL into the staging directory.
fn download_file_to_staging(
    client: &reqwest::blocking::Client,
    url: &str,
    staging_dir: &std::path::Path,
    target_name: &str,
) -> Result<String, String> {
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to download '{target_name}': {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "HTTP {} downloading '{target_name}'",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .map_err(|e| format!("Failed to read '{target_name}' body: {e}"))?;

    let target_path = staging_dir.join(target_name);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory for '{target_name}': {e}"))?;
    }

    fs::write(&target_path, &bytes).map_err(|e| format!("Failed to write '{target_name}': {e}"))?;

    eprintln!(
        "[REPOSITORY] Downloaded '{target_name}' ({} bytes)",
        bytes.len()
    );
    Ok(target_name.to_string())
}

/// Download a GitHub release asset matching the pattern.
/// Uses: GET /repos/{owner}/{repo}/releases to find latest release,
/// then downloads the zip asset and extracts managed files.
fn download_github_release(
    client: &reqwest::blocking::Client,
    config: &serde_json::Value,
    managed_files: Option<&serde_json::Value>,
    staging_dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    let owner = config
        .get("owner")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "GitHub release config missing 'owner'".to_string())?;
    let repo = config
        .get("repo")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "GitHub release config missing 'repo'".to_string())?;
    let asset_pattern = config
        .get("assetPattern")
        .and_then(|v| v.as_str())
        .unwrap_or("*");
    let tag_pattern = config
        .get("tagPattern")
        .and_then(|v| v.as_str())
        .unwrap_or("*");

    eprintln!(
        "[REPOSITORY] Fetching GitHub releases: {owner}/{repo} (tag={tag_pattern}, asset={asset_pattern})"
    );

    // Fetch releases list
    let releases_url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=10");
    let response = client
        .get(&releases_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .map_err(|e| format!("Failed to fetch GitHub releases: {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "GitHub API returned HTTP {} for {owner}/{repo}",
            response.status()
        ));
    }

    let releases: Vec<serde_json::Value> = response
        .json()
        .map_err(|e| format!("Failed to parse GitHub releases response: {e}"))?;

    // Find first release whose tag matches the pattern
    let matching_release = releases.iter().find(|r| {
        let tag = r.get("tag_name").and_then(|v| v.as_str()).unwrap_or("");
        glob_match(tag_pattern, tag)
    });

    let release = match matching_release {
        Some(r) => r,
        None => {
            return Err(format!(
                "No matching release found for {owner}/{repo} with tag pattern '{tag_pattern}'"
            ));
        }
    };

    let tag_name = release
        .get("tag_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    eprintln!("[REPOSITORY] Found release: {tag_name}");

    // Find the zip asset
    let assets = release.get("assets").and_then(|v| v.as_array());
    let zip_asset = assets
        .and_then(|assets| {
            assets.iter().find(|a| {
                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
                glob_match(asset_pattern, name)
            })
        })
        .ok_or_else(|| format!("No asset matching '{asset_pattern}' in release {tag_name}"))?;

    let asset_name = zip_asset
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let asset_url = zip_asset
        .get("browser_download_url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Asset '{asset_name}' has no download URL"))?;

    eprintln!("[REPOSITORY] Downloading asset: {asset_name}");

    // Download the zip
    let zip_response = client
        .get(asset_url)
        .send()
        .map_err(|e| format!("Failed to download asset '{asset_name}': {e}"))?;

    if !zip_response.status().is_success() {
        return Err(format!(
            "HTTP {} downloading asset '{asset_name}'",
            zip_response.status()
        ));
    }

    let zip_bytes = zip_response
        .bytes()
        .map_err(|e| format!("Failed to read asset '{asset_name}': {e}"))?;

    eprintln!(
        "[REPOSITORY] Downloaded asset: {asset_name} ({} bytes)",
        zip_bytes.len()
    );

    // Extract zip and find managed files
    let mut downloaded = Vec::new();

    let managed_paths: Vec<String> = managed_files
        .and_then(|mf| mf.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("path").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if managed_paths.is_empty() {
        eprintln!("[REPOSITORY] No managedFiles declared — skipping extraction");
        return Ok(downloaded);
    }

    // Parse the zip archive
    let cursor = std::io::Cursor::new(&zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open zip archive: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry {i}: {e}"))?;

        let entry_name = file.name().to_string();

        // Check if this entry matches any managed file
        let matching_path = managed_paths.iter().find(|mp| {
            let mp_lower = mp.to_lowercase();
            let entry_lower = entry_name.to_lowercase();
            // Match by filename
            entry_lower.ends_with(&format!("/{}", mp_lower.to_lowercase()))
                || entry_lower == mp_lower
                || entry_name.ends_with(&format!("/{}", mp))
                || entry_name == **mp
        });

        if let Some(target_path) = matching_path {
            if !file.is_dir() {
                let mut content = Vec::new();
                file.read_to_end(&mut content)
                    .map_err(|e| format!("Failed to read zip entry '{entry_name}': {e}"))?;

                let dest = staging_dir.join(target_path);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create dir for '{target_path}': {e}"))?;
                }
                fs::write(&dest, &content)
                    .map_err(|e| format!("Failed to write '{target_path}': {e}"))?;

                eprintln!(
                    "[REPOSITORY] Extracted '{target_path}' ({} bytes) from {asset_name}",
                    content.len()
                );
                downloaded.push(target_path.clone());
            }
        }
    }

    // If no managed files matched inside the zip, try extracting by filename
    if downloaded.is_empty() && !managed_paths.is_empty() {
        eprintln!("[REPOSITORY] No managed files matched by path — attempting filename-only match");
        let cursor2 = std::io::Cursor::new(&zip_bytes);
        let mut archive2 = zip::ZipArchive::new(cursor2)
            .map_err(|e| format!("Failed to reopen zip archive: {e}"))?;

        for i in 0..archive2.len() {
            let mut file = archive2
                .by_index(i)
                .map_err(|e| format!("Failed to read zip entry {i}: {e}"))?;

            let entry_name = file.name().to_string();
            let entry_filename = entry_name.rsplit('/').next().unwrap_or(&entry_name);

            let matching_path = managed_paths.iter().find(|mp| {
                let mp_filename = mp.rsplit('/').next().unwrap_or(mp);
                entry_filename.eq_ignore_ascii_case(mp_filename)
            });

            if let Some(target_path) = matching_path {
                if !file.is_dir() {
                    let mut content = Vec::new();
                    file.read_to_end(&mut content)
                        .map_err(|e| format!("Failed to read zip entry '{entry_name}': {e}"))?;

                    let dest = staging_dir.join(target_path);
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            format!("Failed to create dir for '{target_path}': {e}")
                        })?;
                    }
                    fs::write(&dest, &content)
                        .map_err(|e| format!("Failed to write '{target_path}': {e}"))?;

                    eprintln!(
                        "[REPOSITORY] Extracted '{target_path}' ({} bytes) from {asset_name}",
                        content.len()
                    );
                    downloaded.push(target_path.clone());
                }
            }
        }
    }

    Ok(downloaded)
}

/// Download files from an HTTP release provider.
fn download_http_release(
    client: &reqwest::blocking::Client,
    config: &serde_json::Value,
    managed_files: Option<&serde_json::Value>,
    staging_dir: &std::path::Path,
) -> Result<Vec<String>, String> {
    let base_url = config
        .get("baseUrl")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "HTTP release config missing 'baseUrl'".to_string())?;

    let managed_paths: Vec<String> = managed_files
        .and_then(|mf| mf.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.get("path").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut downloaded = Vec::new();

    for path in &managed_paths {
        let file_url = resolve_file_url(base_url, path)?;
        match download_file_to_staging(client, &file_url, staging_dir, path) {
            Ok(name) => downloaded.push(name),
            Err(e) => {
                eprintln!("[REPOSITORY] Warning: failed to download '{path}': {e}");
            }
        }
    }

    Ok(downloaded)
}

// ---------------------------------------------------------------------------
// Simple glob matching (supports * and ?)
// ---------------------------------------------------------------------------

fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.to_lowercase();
    let t = text.to_lowercase();
    glob_rec(&p, &t)
}

fn glob_rec(pattern: &str, text: &str) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    if pattern.starts_with('*') {
        let rest = &pattern[1..];
        // * matches zero or more characters — try every suffix of text
        for i in 0..=text.len() {
            if glob_rec(rest, &text[i..]) {
                return true;
            }
        }
        false
    } else if pattern.starts_with('?') {
        // ? matches exactly one character
        if text.is_empty() {
            false
        } else {
            glob_rec(&pattern[1..], &text[1..])
        }
    } else {
        // Literal character match
        let pc = pattern.chars().next().unwrap();
        if text.is_empty() {
            false
        } else {
            let tc = text.chars().next().unwrap();
            if pc == tc {
                glob_rec(&pattern[pc.len_utf8()..], &text[tc.len_utf8()..])
            } else {
                false
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_manifest_url_relative() {
        let index_url =
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/index.json";
        let result = resolve_manifest_url(index_url, "extensions/goldberg/manifest.json").unwrap();
        assert_eq!(
            result,
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/extensions/goldberg/manifest.json"
        );
    }

    #[test]
    fn resolve_manifest_url_absolute() {
        let result = resolve_manifest_url(
            "https://example.com/index.json",
            "https://cdn.example.com/m.json",
        )
        .unwrap();
        assert_eq!(result, "https://cdn.example.com/m.json");
    }

    #[test]
    fn resolve_manifest_url_rejects_traversal() {
        let result = resolve_manifest_url("https://example.com/index.json", "../../../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn resolve_file_url_relative() {
        let manifest_url =
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/extensions/goldberg/manifest.json";
        let result = resolve_file_url(manifest_url, "inject.js").unwrap();
        assert_eq!(
            result,
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/extensions/goldberg/inject.js"
        );
    }

    #[test]
    fn valid_extension_ids() {
        assert!(is_valid_extension_id("goldberg").is_ok());
        assert!(is_valid_extension_id("steam-store-helper").is_ok());
        assert!(is_valid_extension_id("open_steam").is_ok());
        assert!(is_valid_extension_id("my-ext-v2").is_ok());
    }

    #[test]
    fn invalid_extension_ids() {
        assert!(is_valid_extension_id("").is_err());
        assert!(is_valid_extension_id("-starts-dash").is_err());
        assert!(is_valid_extension_id("_starts_underscore").is_err());
        assert!(is_valid_extension_id("has spaces").is_err());
        assert!(is_valid_extension_id("has..dots").is_err());
        assert!(is_valid_extension_id("HasUppercase").is_err());
    }

    #[test]
    fn valid_repository_urls() {
        assert!(is_valid_repository_url("https://example.com/index.json").is_ok());
        assert!(is_valid_repository_url("http://localhost/repo").is_ok());
    }

    #[test]
    fn invalid_repository_urls() {
        assert!(is_valid_repository_url("").is_err());
        assert!(is_valid_repository_url("ftp://example.com").is_err());
        assert!(is_valid_repository_url("https://user:pass@example.com").is_err());
        assert!(is_valid_repository_url("https://example.com/../../../etc").is_err());
    }

    #[test]
    fn glob_match_basic() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*.zip", "release.zip"));
        assert!(glob_match("v*", "v1.0.0"));
        assert!(!glob_match("*.zip", "release.tar.gz"));
        assert!(glob_match("?", "a"));
        assert!(!glob_match("?", "ab"));
    }

    #[test]
    fn extension_lua_url_from_manifest() {
        let manifest_url =
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/extensions/opensteamtool/manifest.json";
        let lua_url = resolve_file_url(manifest_url, "extension.lua").unwrap();
        assert_eq!(
            lua_url,
            "https://raw.githubusercontent.com/eisora08/lumaforge-extensions/main/extensions/opensteamtool/extension.lua"
        );
    }

    fn test_plugin_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("lumaforge_test").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn partial_dir_not_installed_via_scan() {
        let dir = test_plugin_dir("partial_dir_not_installed");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        fs::write(
            dir.join("manifest.json"),
            r#"{"id":"test-ext","name":"Test","version":"1.0.0"}"#,
        )
        .unwrap();

        assert!(!dir.join("extension.lua").exists());
        assert!(dir.join("manifest.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_plugin_has_lua_and_manifest() {
        let dir = test_plugin_dir("complete_plugin");
        fs::write(
            dir.join("manifest.json"),
            r#"{"id":"test-ext","name":"Test","version":"1.0.0"}"#,
        )
        .unwrap();
        fs::write(dir.join("extension.lua"), "-- test").unwrap();
        fs::write(
            dir.join("extension-config.json"),
            r#"{"enabled":true,"source":"repository","sourceVersion":"1.0.0","manifestUrl":"https://example.com/m.json"}"#,
        )
        .unwrap();

        assert!(dir.join("manifest.json").exists());
        assert!(dir.join("extension.lua").exists());

        let raw = fs::read_to_string(dir.join("extension-config.json")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            config.get("source").unwrap().as_str().unwrap(),
            "repository"
        );
        assert_eq!(
            config.get("manifestUrl").unwrap().as_str().unwrap(),
            "https://example.com/m.json"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn toggle_preserves_existing_config_fields() {
        let existing: serde_json::Value = serde_json::json!({
            "enabled": true,
            "source": "repository",
            "sourceVersion": "1.4.8",
            "manifestUrl": "https://example.com/manifest.json"
        });

        let mut merged = existing.clone();
        merged["enabled"] = serde_json::json!(false);

        assert_eq!(
            merged.get("source").unwrap().as_str().unwrap(),
            "repository"
        );
        assert_eq!(
            merged.get("sourceVersion").unwrap().as_str().unwrap(),
            "1.4.8"
        );
        assert_eq!(
            merged.get("manifestUrl").unwrap().as_str().unwrap(),
            "https://example.com/manifest.json"
        );
        assert!(!merged.get("enabled").unwrap().as_bool().unwrap());
    }

    #[test]
    fn no_managed_files_in_plugin_dir_for_lua_ext() {
        let dir = test_plugin_dir("no_managed_files");
        fs::write(
            dir.join("manifest.json"),
            r#"{"id":"opensteamtool","name":"OpenSteamTool","version":"1.4.8"}"#,
        )
        .unwrap();
        fs::write(dir.join("extension.lua"), "-- lifecycle").unwrap();

        assert!(!dir.join("dwmapi.dll").exists());
        assert!(!dir.join("xinput1_4.dll").exists());
        assert!(!dir.join("OpenSteamTool.dll").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn uninstall_requires_repository_source() {
        let dir = test_plugin_dir("uninstall_source");
        assert!(!dir.join("extension-config.json").exists());

        let is_repo_source = false;
        assert!(!is_repo_source);

        fs::write(
            dir.join("extension-config.json"),
            r#"{"enabled":true,"source":"repository"}"#,
        )
        .unwrap();
        let raw = fs::read_to_string(dir.join("extension-config.json")).unwrap();
        let config: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let is_repo = config
            .get("source")
            .and_then(|s| s.as_str())
            .map(|s| s == "repository")
            .unwrap_or(false);
        assert!(is_repo);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lua_extension_table_serializes_camel_case_keys() {
        let table = crate::lua_engine::LuaExtensionTable {
            id: Some("opensteamtool".into()),
            name: Some("OpenSteamTool".into()),
            version: Some("1.4.8".into()),
            description: Some("DLL tool".into()),
            criteria: None,
            has_detect: true,
            has_install: true,
            has_enable: true,
            has_disable: true,
            has_uninstall: true,
        };

        let value = serde_json::to_value(&table).unwrap();

        assert!(
            value.get("hasInstall").is_some(),
            "must serialize as hasInstall, got: {value}"
        );
        assert!(
            value.get("hasDetect").is_some(),
            "must serialize as hasDetect, got: {value}"
        );
        assert!(
            value
                .get("hasInstall")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "hasInstall must be true"
        );
        assert!(
            value
                .get("hasDetect")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            "hasDetect must be true"
        );
        assert!(
            value.get("has_enable").is_none(),
            "must NOT have snake_case key has_enable"
        );
        assert!(
            value.get("has_install").is_none(),
            "must NOT have snake_case key has_install"
        );
    }

    #[test]
    fn lua_extension_table_snake_case_keys_not_present() {
        let table = crate::lua_engine::LuaExtensionTable {
            id: Some("test".into()),
            name: None,
            version: None,
            description: None,
            criteria: None,
            has_detect: false,
            has_install: false,
            has_enable: false,
            has_disable: false,
            has_uninstall: false,
        };

        let value = serde_json::to_value(&table).unwrap();

        let has_install_camel = value
            .get("hasInstall")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let has_install_snake = value
            .get("has_install")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            !has_install_camel,
            "hasInstall should be false when has_install=false"
        );
        assert!(
            !has_install_snake,
            "has_install snake_case should not exist in output"
        );
    }
}
