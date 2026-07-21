use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

use super::hubcap;

const RYUU_BASE_URL: &str = "https://generator.ryuu.lol";
const RYUU_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// Provider adapter trait
// ---------------------------------------------------------------------------

/// Result of a provider availability check.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAvailability {
    pub provider_id: String,
    pub name: String,
    pub available: bool,
    pub file_count: u32,
    pub total_size: u64,
    pub detail: Option<String>,
    pub usage: Option<hubcap::HubcapUsageStats>,
}

/// Trait for provider adapters that can check availability and download.
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn check_availability(&self, app_id: &str) -> ProviderAvailability;
}

// ---------------------------------------------------------------------------
// HubcapDB adapter
// ---------------------------------------------------------------------------

pub struct HubcapDBAdapter;

impl ProviderAdapter for HubcapDBAdapter {
    fn id(&self) -> &str {
        "hubcapdb"
    }

    fn name(&self) -> &str {
        "HubcapDB"
    }

    fn check_availability(&self, app_id: &str) -> ProviderAvailability {
        let result = hubcap::check_availability(app_id);

        let usage = match hubcap::get_user_stats() {
            Ok(stats) => Some(stats),

            Err(error) => {
                eprintln!("[HUBCAP] Usage statistics unavailable: {error}");

                None
            }
        };

        ProviderAvailability {
            provider_id: "hubcapdb".to_string(),
            name: self.name().to_string(),
            available: result.available,
            file_count: result.file_count,
            total_size: result.total_size,
            detail: result.detail,
            usage,
        }
    }
}

// ---------------------------------------------------------------------------
// Ryuu adapter
// ---------------------------------------------------------------------------

pub struct RyuuAdapter;

fn ryuu_build_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(RYUU_TIMEOUT_SECS))
        .danger_accept_invalid_certs(false)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn ryuu_get_api_key() -> Option<String> {
    let config = crate::config::load_config();
    config
        .downloads
        .providers
        .iter()
        .find(|p| p.id == "ryuu")
        .and_then(|p| p.api_key.clone())
        .filter(|k| !k.is_empty())
}

impl ProviderAdapter for RyuuAdapter {
    fn id(&self) -> &str {
        "ryuu"
    }

    fn name(&self) -> &str {
        "Ryuu"
    }

    fn check_availability(&self, _app_id: &str) -> ProviderAvailability {
        let detail = if ryuu_get_api_key().is_some() {
            "Ryuu does not provide a separate availability endpoint."
        } else {
            "No API key configured."
        };

        ProviderAvailability {
            provider_id: "ryuu".to_string(),
            name: self.name().to_string(),
            available: false,
            usage: None,
            file_count: 0,
            total_size: 0,
            detail: Some(detail.to_string()),
        }
    }
}

fn ryuu_get_base_url() -> String {
    let config = crate::config::load_config();

    config
        .downloads
        .providers
        .iter()
        .find(|provider| provider.id == "ryuu")
        .map(|provider| provider.base_url.clone())
        .unwrap_or_else(|| RYUU_BASE_URL.to_string())
}

/// Download a package from Ryuu. Returns the raw ZIP bytes and SHA-256 hex.
pub fn ryuu_download(app_id: &str) -> Result<Vec<u8>, String> {
    let api_key = ryuu_get_api_key().ok_or_else(|| "No Ryuu API key configured.".to_string())?;
    let base_url = ryuu_get_base_url();
    let url = format!("{base_url}/api/download/{app_id}");
    let client = ryuu_build_client();

    eprintln!("[RYUU] Downloading package for app_id={app_id} from {url}");

    let resp = client
        .get(&url)
        .header("X-Auth-Key", &api_key)
        .send()
        .map_err(|e| format!("Ryuu download request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(format!("Ryuu download failed with HTTP {status}: {text}"));
    }

    let bytes = resp
        .bytes()
        .map_err(|e| format!("Failed to read Ryuu download response: {e}"))?;

    eprintln!(
        "[RYUU] Downloaded {} bytes for app_id={app_id}",
        bytes.len()
    );

    Ok(bytes.to_vec())
}

// ---------------------------------------------------------------------------
// Adapter registry
// ---------------------------------------------------------------------------

fn builtin_adapters() -> &'static Vec<Box<dyn ProviderAdapter>> {
    static ADAPTERS: OnceLock<Vec<Box<dyn ProviderAdapter>>> = OnceLock::new();
    ADAPTERS.get_or_init(|| vec![Box::new(HubcapDBAdapter), Box::new(RyuuAdapter)])
}

/// Find an adapter by provider ID.
pub fn find_adapter(provider_id: &str) -> Option<&'static Box<dyn ProviderAdapter>> {
    builtin_adapters().iter().find(|a| a.id() == provider_id)
}

// ---------------------------------------------------------------------------
// Availability check for all enabled providers
// ---------------------------------------------------------------------------

/// Check availability for a specific app across all enabled providers.
/// Returns a list of ProviderAvailability results, one per enabled provider.
pub fn check_sources_availability(app_id: &str) -> Vec<ProviderAvailability> {
    let config = crate::config::load_config();
    let enabled_providers: Vec<String> = config
        .downloads
        .providers
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.id.clone())
        .collect();

    let mut results = Vec::new();

    for provider_id in &enabled_providers {
        let availability = if let Some(adapter) = find_adapter(provider_id) {
            adapter.check_availability(app_id)
        } else {
            ProviderAvailability {
                provider_id: provider_id.clone(),
                name: provider_id.clone(),
                available: false,
                file_count: 0,
                usage: None,
                total_size: 0,
                detail: Some("No adapter available".to_string()),
            }
        };
        results.push(availability);
    }

    results
}

// ---------------------------------------------------------------------------
// Multi-provider fallback download
// ---------------------------------------------------------------------------

/// Attempt download with multi-provider fallback.
/// Tries providers in config order; returns the first successful result.
pub fn download_with_fallback(
    app_id: &str,
    preferred_provider: &str,
    steam_root: &std::path::Path,
    request_id: Option<&str>,
    output_type: Option<&str>,
) -> Result<hubcap::ExtractionResult, String> {
    let config = crate::config::load_config();
    let multi_fallback = config.downloads.multi_provider_fallback;

    // Build ordered list: preferred first, then rest
    let mut ordered_ids: Vec<String> = config
        .downloads
        .providers
        .iter()
        .filter(|p| p.enabled)
        .map(|p| p.id.clone())
        .collect();

    // Move preferred to front
    if let Some(pos) = ordered_ids.iter().position(|id| id == preferred_provider) {
        let id = ordered_ids.remove(pos);
        ordered_ids.insert(0, id);
    }

    let mut last_error = String::new();

    for provider_id in &ordered_ids {
        let is_preferred = provider_id == preferred_provider;

        // A fallback provider must confirm availability before it is used.
        // The provider explicitly selected by the user is handled normally.
        if !is_preferred {
            let Some(adapter) = find_adapter(provider_id) else {
                last_error = format!("Provider '{provider_id}' has no adapter.");

                eprintln!(
                    "[DOWNLOADS] Skipping fallback provider '{}': {}",
                    provider_id, last_error
                );

                continue;
            };

            let availability = adapter.check_availability(app_id);

            if !availability.available {
                last_error = availability.detail.unwrap_or_else(|| {
                    format!("Provider '{provider_id}' did not confirm package availability.")
                });

                eprintln!(
                    "[DOWNLOADS] Skipping fallback provider '{}': {}",
                    provider_id, last_error
                );

                continue;
            }

            eprintln!(
                "[DOWNLOADS] Fallback provider '{}' confirmed availability for app_id={}",
                provider_id, app_id
            );
        }

        // Only hubcapdb and ryuu have real adapters for now
        if provider_id == "hubcapdb" {
            match hubcap::download_and_extract(app_id, steam_root, request_id, output_type) {
                Ok(result) => {
                    let processed_anything =
                        !result.lua_files.is_empty() || !result.manifest_files.is_empty();

                    if !result.errors.is_empty() {
                        last_error = format!(
                            "Package processing failed via {provider_id}: {}",
                            result.errors.join("; ")
                        );

                        eprintln!("[DOWNLOADS] {last_error}");

                        if !multi_fallback {
                            return Err(last_error);
                        }

                        continue;
                    }

                    if !processed_anything {
                        last_error = format!(
            "Package from provider '{provider_id}' contained no supported files for output type '{}'.",
            output_type.unwrap_or("lua+manifest")
        );

                        eprintln!("[DOWNLOADS] {last_error}");

                        if !multi_fallback {
                            return Err(last_error);
                        }

                        continue;
                    }

                    eprintln!(
        "[DOWNLOADS] Package processed successfully via {provider_id} for app_id={app_id}: {} Lua file(s), {} manifest file(s)",
        result.lua_files.len(),
        result.manifest_files.len()
    );

                    return Ok(result);
                }

                Err(error) => {
                    eprintln!("[DOWNLOADS] Download failed via {provider_id}: {error}");

                    last_error = error;

                    if !multi_fallback {
                        return Err(last_error);
                    }

                    continue;
                }
            }
        } else if provider_id == "ryuu" {
            match ryuu_download(app_id) {
                Ok(bytes) => {
                    match extract_ryuu_zip(&bytes, app_id, steam_root, request_id, output_type) {
                        Ok(result) => {
                            let processed_anything =
                                !result.lua_files.is_empty() || !result.manifest_files.is_empty();

                            if !result.errors.is_empty() {
                                last_error = format!(
                                    "Package processing failed via {provider_id}: {}",
                                    result.errors.join("; ")
                                );

                                eprintln!("[DOWNLOADS] {last_error}");

                                if !multi_fallback {
                                    return Err(last_error);
                                }

                                continue;
                            }

                            if !processed_anything {
                                last_error = format!(
            "Package from provider '{provider_id}' contained no supported files for output type '{}'.",
            output_type.unwrap_or("lua+manifest")
        );

                                eprintln!("[DOWNLOADS] {last_error}");

                                if !multi_fallback {
                                    return Err(last_error);
                                }

                                continue;
                            }

                            eprintln!(
        "[DOWNLOADS] Package processed successfully via {provider_id} for app_id={app_id}: {} Lua file(s), {} manifest file(s)",
        result.lua_files.len(),
        result.manifest_files.len()
    );

                            return Ok(result);
                        }
                        Err(error) => {
                            eprintln!("[DOWNLOADS] Extraction failed via {provider_id}: {error}");

                            last_error = error;

                            if !multi_fallback {
                                return Err(last_error);
                            }
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[DOWNLOADS] Download failed via {provider_id}: {error}");

                    last_error = error;

                    if !multi_fallback {
                        return Err(last_error);
                    }
                }
            }
        } else {
            eprintln!("[DOWNLOADS] Provider {provider_id} has no adapter, skipping");

            last_error = format!("Provider '{provider_id}' does not have a download adapter yet.");

            if !multi_fallback {
                return Err(last_error);
            }
        }
    }

    if last_error.is_empty() {
        Err("No enabled provider confirmed package availability.".to_string())
    } else {
        Err(last_error)
    }
}

/// Extract a Ryuu ZIP (same logic as hubcap but from raw bytes).
fn extract_ryuu_zip(
    bytes: &[u8],
    app_id: &str,
    steam_root: &std::path::Path,
    request_id: Option<&str>,
    output_type: Option<&str>,
) -> Result<hubcap::ExtractionResult, String> {
    use std::io::Read;

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open Ryuu ZIP: {e}"))?;

    let lua_dir = steam_root.join("config").join("lua");
    let depot_dir = steam_root.join("depotcache");
    let backup_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("LumaForge")
        .join("backup")
        .join(app_id);

    std::fs::create_dir_all(&lua_dir)
        .map_err(|e| format!("Failed to create config/lua directory: {e}"))?;
    std::fs::create_dir_all(&depot_dir)
        .map_err(|e| format!("Failed to create depotcache directory: {e}"))?;

    let mut result = hubcap::ExtractionResult {
        lua_files: Vec::new(),
        manifest_files: Vec::new(),
        errors: Vec::new(),
        lua_sha256: None,
        manifest_sha256: None,
    };

    let mut lua_bytes_all: Vec<u8> = Vec::new();
    let mut manifest_bytes_all: Vec<u8> = Vec::new();

    let extract_lua = output_type.map_or(true, |t| t == "lua" || t == "lua+manifest");
    let extract_manifest = output_type.map_or(true, |t| t == "manifest" || t == "lua+manifest");

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(e) => {
                result
                    .errors
                    .push(format!("Failed to read Ryuu ZIP entry {i}: {e}"));
                continue;
            }
        };

        let entry_name = entry.name().to_string();
        let lower_name = entry_name.to_lowercase();

        if extract_lua && lower_name.ends_with(".lua") {
            let file_name = std::path::Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = lua_dir.join(&file_name);

            // Backup existing
            super::hubcap::backup_lua_file(&backup_dir, &dest);

            let mut content = Vec::new();
            if entry.read_to_end(&mut content).is_ok() {
                if std::fs::write(&dest, &content).is_ok() {
                    eprintln!("[RYUU] Extracted LUA: {}", dest.display());
                    result.lua_files.push(dest.to_string_lossy().to_string());
                    lua_bytes_all.extend_from_slice(&content);
                }
            }
        } else if extract_manifest && lower_name.ends_with(".manifest") {
            let file_name = std::path::Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = depot_dir.join(&file_name);

            let mut content = Vec::new();
            if entry.read_to_end(&mut content).is_ok() {
                if std::fs::write(&dest, &content).is_ok() {
                    eprintln!("[RYUU] Extracted MANIFEST: {}", dest.display());
                    result
                        .manifest_files
                        .push(dest.to_string_lossy().to_string());
                    manifest_bytes_all.extend_from_slice(&content);
                }
            }
        }
    }

    if !lua_bytes_all.is_empty() {
        result.lua_sha256 = Some(hubcap::sha256_hex(&lua_bytes_all));
    }
    if !manifest_bytes_all.is_empty() {
        result.manifest_sha256 = Some(hubcap::sha256_hex(&manifest_bytes_all));
    }

    // Save metadata
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string());

    let metadata = hubcap::PackageMetadata {
        app_id: app_id.to_string(),
        provider_id: "ryuu".to_string(),
        request_id: request_id.map(|s| s.to_string()),
        downloaded_at: now,
        lua_sha256: result.lua_sha256.clone(),
        manifest_sha256: result.manifest_sha256.clone(),
        lua_files: result.lua_files.clone(),
        manifest_files: result.manifest_files.clone(),
    };
    if let Err(e) = hubcap::save_package_metadata(app_id, &metadata) {
        result.errors.push(format!("Failed to save metadata: {e}"));
    }

    // Clean up temp dir
    if let Some(rid) = request_id {
        let temp_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("LumaForge")
            .join("downloads")
            .join("temp")
            .join(rid);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hubcapdb_adapter_id() {
        let a = HubcapDBAdapter;
        assert_eq!(a.id(), "hubcapdb");
        assert_eq!(a.name(), "HubcapDB");
    }

    #[test]
    fn ryuu_adapter_id() {
        let a = RyuuAdapter;
        assert_eq!(a.id(), "ryuu");
        assert_eq!(a.name(), "Ryuu");
    }

    #[test]
    fn builtin_adapters_has_two() {
        let adapters = builtin_adapters();
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].id(), "hubcapdb");
        assert_eq!(adapters[1].id(), "ryuu");
    }

    #[test]
    fn find_adapter_hubcapdb() {
        let a = find_adapter("hubcapdb");
        assert!(a.is_some());
        assert_eq!(a.unwrap().id(), "hubcapdb");
    }

    #[test]
    fn find_adapter_ryuu() {
        let a = find_adapter("ryuu");
        assert!(a.is_some());
        assert_eq!(a.unwrap().id(), "ryuu");
    }

    #[test]
    fn find_adapter_unknown_returns_none() {
        assert!(find_adapter("nonexistent").is_none());
    }

    #[test]
    fn provider_availability_serializes() {
        let pa = ProviderAvailability {
            provider_id: "hubcapdb".into(),
            name: "HubcapDB".into(),
            available: true,
            file_count: 5,
            total_size: 1024,
            usage: None,
            detail: None,
        };
        let json = serde_json::to_string(&pa).unwrap();
        assert!(json.contains("hubcapdb"));
        assert!(json.contains("true"));
    }

    #[test]
    fn check_sources_availability_returns_results() {
        let results = check_sources_availability("730");
        // Should return at least one result (hubcapdb is enabled by default)
        assert!(!results.is_empty());
        // First result should be hubcapdb
        assert_eq!(results[0].provider_id, "hubcapdb");
    }

    #[test]
    fn ryuu_download_fails_without_key() {
        // Without a configured key, should return error
        let result = ryuu_download("730");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("API key") || err.contains("No Ryuu"));
    }
}
