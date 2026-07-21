use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const HUBCAP_BASE_URL: &str = "https://hubcapmanifest.com/api/v1";
const HUBCAP_TIMEOUT_SECS: u64 = 30;
const MAX_ZIP_SIZE_BYTES: u64 = 500 * 1024 * 1024; // 500 MB
const MAX_ZIP_ENTRIES: usize = 10_000;
const MAX_SINGLE_ENTRY_SIZE: u64 = 200 * 1024 * 1024; // 200 MB

// ---------------------------------------------------------------------------
// API Key storage (in-memory, persisted via AppConfig)
// ---------------------------------------------------------------------------

static API_KEY: Mutex<Option<String>> = Mutex::new(None);

pub fn get_api_key() -> Option<String> {
    {
        let guard = API_KEY.lock().unwrap();
        if let Some(ref key) = *guard {
            if !key.is_empty() {
                return Some(key.clone());
            }
        }
    }
    let config = crate::config::load_config();
    let key = config
        .downloads
        .providers
        .iter()
        .find(|p| p.id == "hubcapdb")
        .and_then(|p| p.api_key.clone())
        .filter(|k| !k.is_empty());
    if let Some(ref k) = key {
        let mut guard = API_KEY.lock().unwrap();
        *guard = Some(k.clone());
    }
    key
}

#[allow(dead_code)]
pub fn set_api_key(key: String) {
    let mut guard = API_KEY.lock().unwrap();
    *guard = Some(key);
}

#[allow(dead_code)]
pub fn has_api_key() -> bool {
    get_api_key().is_some()
}

fn build_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(HUBCAP_TIMEOUT_SECS))
        .danger_accept_invalid_certs(false)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

// ---------------------------------------------------------------------------
// Path helpers
// ---------------------------------------------------------------------------

/// Canonical AppData base directory: `{local_data_dir}/LumaForge`
fn app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("LumaForge")
}

/// Temp download directory for a specific request.
fn temp_dir_for_request(request_id: &str) -> PathBuf {
    app_data_dir()
        .join("downloads")
        .join("temp")
        .join(request_id)
}

/// Package metadata directory for a specific app.
fn package_metadata_dir(app_id: &str) -> PathBuf {
    app_data_dir().join("packages").join(app_id)
}

/// Canonical Lua destination under Steam root.
fn lua_dest_dir(steam_root: &Path) -> PathBuf {
    steam_root.join("config").join("lua")
}

/// Canonical manifest destination under Steam root.
fn manifest_dest_dir(steam_root: &Path) -> PathBuf {
    steam_root.join("depotcache")
}

// ---------------------------------------------------------------------------
// ZIP safety validation
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ZipSafetyReport {
    has_traversal: bool,
    has_too_large_entry: bool,
    total_decompressed_hint: u64,
}

fn validate_zip_safety(
    archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>,
) -> Result<ZipSafetyReport, String> {
    let entry_count = archive.len();
    if entry_count > MAX_ZIP_ENTRIES {
        return Err(format!(
            "ZIP contains {entry_count} entries, exceeds limit of {MAX_ZIP_ENTRIES}"
        ));
    }

    let mut report = ZipSafetyReport {
        has_traversal: false,
        has_too_large_entry: false,
        total_decompressed_hint: 0,
    };

    for i in 0..entry_count {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {i}: {e}"))?;
        let name = entry.name().to_string();

        // Check for path traversal
        if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
            report.has_traversal = true;
            eprintln!("[HUBCAP] ZIP traversal detected in entry: {name}");
        }

        let lower = name.to_lowercase();
        let is_lua = lower.ends_with(".lua");
        let is_manifest = lower.ends_with(".manifest");
        if !is_lua && !is_manifest {
            continue;
        }

        // Check uncompressed size
        let size = entry.size();
        report.total_decompressed_hint += size;
        if size > MAX_SINGLE_ENTRY_SIZE {
            report.has_too_large_entry = true;
        }
    }

    if report.has_traversal {
        return Err("ZIP archive contains path-traversal entries".to_string());
    }
    if report.has_too_large_entry {
        return Err(format!(
            "ZIP contains an entry exceeding {MAX_SINGLE_ENTRY_SIZE} bytes"
        ));
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// SHA-256 checksum helpers
// ---------------------------------------------------------------------------

/// Compute SHA-256 hex digest of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Lua backup
// ---------------------------------------------------------------------------

/// Back up an existing Lua file before replacement.
/// Creates `backup/` dir next to the Lua dest dir, copies the existing file there.
pub fn backup_lua_file(backup_dir: &Path, lua_file: &Path) {
    if !lua_file.exists() {
        return;
    }
    let file_name = match lua_file.file_name() {
        Some(n) => n.to_owned(),
        None => return,
    };
    let _ = fs::create_dir_all(backup_dir);
    let dest = backup_dir.join(&file_name);
    let _ = fs::copy(lua_file, &dest);
    eprintln!(
        "[HUBCAP] Backed up {} to {}",
        lua_file.display(),
        dest.display()
    );
}

// ---------------------------------------------------------------------------
// Package metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMetadata {
    pub app_id: String,
    pub provider_id: String,
    pub request_id: Option<String>,
    pub downloaded_at: String,
    pub lua_sha256: Option<String>,
    pub manifest_sha256: Option<String>,
    pub lua_files: Vec<String>,
    pub manifest_files: Vec<String>,
}

/// Save package metadata to disk.
pub fn save_package_metadata(app_id: &str, metadata: &PackageMetadata) -> Result<(), String> {
    let dir = package_metadata_dir(app_id);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create metadata dir: {e}"))?;
    let path = dir.join("metadata.json");
    let json = serde_json::to_string_pretty(metadata)
        .map_err(|e| format!("Failed to serialize metadata: {e}"))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write metadata: {e}"))?;
    eprintln!(
        "[HUBCAP] Saved metadata for app_id={app_id} to {}",
        path.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Availability check
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailabilityResult {
    pub available: bool,
    pub file_count: u32,
    pub total_size: u64,
    pub detail: Option<String>,
}

/// Check if HubcapDB has a package available for the given app_id.
/// Calls `GET /api/v1/status/{app_id}`.
pub fn check_availability(app_id: &str) -> AvailabilityResult {
    let api_key = match get_api_key() {
        Some(k) => k,
        None => {
            return AvailabilityResult {
                available: false,
                file_count: 0,
                total_size: 0,
                detail: Some("No API key configured".to_string()),
            }
        }
    };

    let url = format!("{HUBCAP_BASE_URL}/status/{app_id}");
    let client = build_client();

    match client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
    {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                eprintln!("[HUBCAP] Availability check failed for {app_id}: HTTP {status}");

                return AvailabilityResult {
                    available: false,
                    file_count: 0,
                    total_size: 0,
                    detail: Some(format!("HTTP {status}")),
                };
            }
            match resp.json::<serde_json::Value>() {
                Ok(value) => {
                    let provider_status = value
                        .get("status")
                        .and_then(|field| field.as_str())
                        .unwrap_or("unknown");

                    let manifest_file_exists = value
                        .get("manifest_file_exists")
                        .and_then(|field| field.as_bool())
                        .unwrap_or(false);

                    let update_in_progress = value
                        .get("update_in_progress")
                        .and_then(|field| field.as_bool())
                        .unwrap_or(false);

                    let available =
                        provider_status.eq_ignore_ascii_case("available") && manifest_file_exists;

                    let file_count = if manifest_file_exists { 1 } else { 0 };

                    let total_size = value
                        .get("file_size")
                        .and_then(|field| field.as_u64())
                        .unwrap_or(0);

                    let game_name = value
                        .get("game_name")
                        .and_then(|field| field.as_str())
                        .unwrap_or("Unknown game");

                    let detail = if available {
                        Some(format!(
                            "{game_name} • {} bytes{}",
                            total_size,
                            if update_in_progress {
                                " • update in progress"
                            } else {
                                ""
                            }
                        ))
                    } else if update_in_progress {
                        Some(format!(
                            "{game_name} • Package update is currently in progress"
                        ))
                    } else {
                        Some(format!("{game_name} • Package status: {provider_status}"))
                    };

                    eprintln!(
            "[HUBCAP] Availability for {app_id}: status={provider_status}, manifest_exists={manifest_file_exists}, update_in_progress={update_in_progress}, available={available}, size={total_size}"
        );

                    AvailabilityResult {
                        available,
                        file_count,
                        total_size,
                        detail,
                    }
                }
                Err(e) => {
                    eprintln!("[HUBCAP] Failed to parse availability response: {e}");
                    AvailabilityResult {
                        available: false,
                        file_count: 0,
                        total_size: 0,
                        detail: Some(format!("Parse error: {e}")),
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[HUBCAP] Availability request failed for {app_id}: {e}");
            AvailabilityResult {
                available: false,
                file_count: 0,
                total_size: 0,
                detail: Some(format!("Network error: {e}")),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Download & extraction (enhanced with safety, SHA-256, temp dirs, backup)
// ---------------------------------------------------------------------------

/// Download a package ZIP from Hubcap and extract to the Steam directories.
/// - Downloads to temp dir first
/// - Validates ZIP safety (traversal, size, count)
/// - Computes SHA-256 of the ZIP
/// - Backs up existing .lua files before replacement
/// - Extracts `.lua` files → `{steam_root}/config/lua/`
/// - Extracts `.manifest` files → `{steam_root}/depotcache/`
/// - Saves package metadata to AppData
pub fn download_and_extract(
    app_id: &str,
    steam_root: &Path,
    request_id: Option<&str>,
    output_type: Option<&str>,
) -> Result<ExtractionResult, String> {
    let api_key = get_api_key().ok_or_else(|| "No Hubcap API key configured.".to_string())?;

    let url = format!("{HUBCAP_BASE_URL}/manifest/{app_id}");
    let client = build_client();

    eprintln!("[DOWNLOADS] Starting HubcapDB download for app_id={app_id}");

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .map_err(|e| format!("Download request failed: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().unwrap_or_default();
        return Err(format!("Download failed with HTTP {status}: {text}"));
    }

    let bytes = resp
        .bytes()
        .map_err(|e| format!("Failed to read download response: {e}"))?;

    let zip_size = bytes.len() as u64;
    if zip_size > MAX_ZIP_SIZE_BYTES {
        return Err(format!(
            "ZIP archive is {zip_size} bytes, exceeds limit of {MAX_ZIP_SIZE_BYTES}"
        ));
    }

    eprintln!("[DOWNLOADS] Downloaded {zip_size} bytes for app_id={app_id}");

    // Compute SHA-256 of the raw ZIP bytes
    let zip_sha256 = sha256_hex(&bytes);
    eprintln!("[DOWNLOADS] ZIP SHA-256: {zip_sha256}");

    // Save to temp directory
    let temp_dir = match request_id {
        Some(rid) => {
            let td = temp_dir_for_request(rid);
            let _ = fs::create_dir_all(&td);
            let zip_path = td.join(format!("{app_id}.zip"));
            fs::write(&zip_path, &bytes)
                .map_err(|e| format!("Failed to save ZIP to temp dir: {e}"))?;
            eprintln!("[DOWNLOADS] Saved ZIP to temp: {}", zip_path.display());
            Some(td)
        }
        None => None,
    };

    // Open and validate ZIP
    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open ZIP archive: {e}"))?;

    let _report = validate_zip_safety(&mut archive)?;

    // Prepare destination directories
    let lua_dir = lua_dest_dir(steam_root);
    let depot_dir = manifest_dest_dir(steam_root);
    let backup_dir = app_data_dir().join("backup").join(app_id);

    fs::create_dir_all(&lua_dir)
        .map_err(|e| format!("Failed to create config/lua directory: {e}"))?;
    fs::create_dir_all(&depot_dir)
        .map_err(|e| format!("Failed to create depotcache directory: {e}"))?;

    let mut result = ExtractionResult {
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
                    .push(format!("Failed to read ZIP entry {i}: {e}"));
                continue;
            }
        };

        let entry_name = entry.name().to_string();
        let lower_name = entry_name.to_lowercase();

        if extract_lua && lower_name.ends_with(".lua") {
            let file_name = Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = lua_dir.join(&file_name);

            // Backup existing before overwrite
            backup_lua_file(&backup_dir, &dest);

            let mut content = Vec::new();
            match entry.read_to_end(&mut content) {
                Ok(_) => match fs::write(&dest, &content) {
                    Ok(_) => {
                        eprintln!("[HUBCAP] Extracted LUA: {}", dest.display());
                        result.lua_files.push(dest.to_string_lossy().to_string());
                        lua_bytes_all.extend_from_slice(&content);
                    }
                    Err(e) => {
                        result
                            .errors
                            .push(format!("Failed to write {}: {e}", dest.display()));
                    }
                },
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to read LUA entry {entry_name}: {e}"));
                }
            }
        } else if extract_manifest && lower_name.ends_with(".manifest") {
            let file_name = Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = depot_dir.join(&file_name);

            let mut content = Vec::new();
            match entry.read_to_end(&mut content) {
                Ok(_) => match fs::write(&dest, &content) {
                    Ok(_) => {
                        eprintln!("[HUBCAP] Extracted MANIFEST: {}", dest.display());
                        result
                            .manifest_files
                            .push(dest.to_string_lossy().to_string());
                        manifest_bytes_all.extend_from_slice(&content);
                    }
                    Err(e) => {
                        result
                            .errors
                            .push(format!("Failed to write {}: {e}", dest.display()));
                    }
                },
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to read MANIFEST entry {entry_name}: {e}"));
                }
            }
        }
    }

    // Compute combined SHA-256 for extracted content
    if !lua_bytes_all.is_empty() {
        result.lua_sha256 = Some(sha256_hex(&lua_bytes_all));
    }
    if !manifest_bytes_all.is_empty() {
        result.manifest_sha256 = Some(sha256_hex(&manifest_bytes_all));
    }

    // Save package metadata
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| format!("{}", d.as_secs()))
        .unwrap_or_else(|_| "0".to_string());

    let metadata = PackageMetadata {
        app_id: app_id.to_string(),
        provider_id: "hubcapdb".to_string(),
        request_id: request_id.map(|s| s.to_string()),
        downloaded_at: now,
        lua_sha256: result.lua_sha256.clone(),
        manifest_sha256: result.manifest_sha256.clone(),
        lua_files: result.lua_files.clone(),
        manifest_files: result.manifest_files.clone(),
    };
    if let Err(e) = save_package_metadata(app_id, &metadata) {
        result.errors.push(format!("Failed to save metadata: {e}"));
    }

    // Clean up temp directory
    if let Some(td) = temp_dir {
        let _ = fs::remove_dir_all(&td);
    }

    eprintln!(
        "[DOWNLOADS] Extraction complete for app_id={app_id}: {} LUA, {} manifest, {} errors, zip_sha256={}",
        result.lua_files.len(),
        result.manifest_files.len(),
        result.errors.len(),
        &zip_sha256[..16],
    );

    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub lua_files: Vec<String>,
    pub manifest_files: Vec<String>,
    pub errors: Vec<String>,
    pub lua_sha256: Option<String>,
    pub manifest_sha256: Option<String>,
}

// ---------------------------------------------------------------------------
// Local status check
// ---------------------------------------------------------------------------

/// Check if a lua file exists for the given app_id in the Steam config/lua/ directory.
pub fn check_local_status(app_id: &str, steam_root: &Path) -> bool {
    let lua_path = lua_dest_dir(steam_root).join(format!("{app_id}.lua"));
    lua_path.exists()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_status_returns_false_for_missing() {
        let tmp = std::env::temp_dir().join("luma_test_nonexistent");
        let result = check_local_status("99999", &tmp);
        assert!(!result);
    }

    #[test]
    fn local_status_returns_true_when_file_exists() {
        let tmp = std::env::temp_dir().join("luma_test_local_status");
        let lua_dir = tmp.join("config").join("lua");
        fs::create_dir_all(&lua_dir).unwrap();
        fs::write(lua_dir.join("12345.lua"), "-- test").unwrap();

        assert!(check_local_status("12345", &tmp));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extraction_result_serializes() {
        let r = ExtractionResult {
            lua_files: vec!["test.lua".into()],
            manifest_files: vec!["test.manifest".into()],
            errors: vec![],
            lua_sha256: None,
            manifest_sha256: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("test.lua"));
        assert!(json.contains("test.manifest"));
    }

    #[test]
    fn sha256_hex_produces_64_chars() {
        let hash = sha256_hex(b"hello world");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let a = sha256_hex(b"test data");
        let b = sha256_hex(b"test data");
        assert_eq!(a, b);
    }

    #[test]
    fn sha256_hex_differs_for_different_data() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn backup_lua_file_creates_backup() {
        let tmp = std::env::temp_dir().join("luma_test_backup");
        let _ = fs::remove_dir_all(&tmp);
        let lua_dir = tmp.join("config").join("lua");
        fs::create_dir_all(&lua_dir).unwrap();
        let lua_file = lua_dir.join("12345.lua");
        fs::write(&lua_file, "-- original").unwrap();

        let backup_dir = tmp.join("backup");
        backup_lua_file(&backup_dir, &lua_file);

        let backed_up = backup_dir.join("12345.lua");
        assert!(backed_up.exists());
        assert_eq!(fs::read_to_string(&backed_up).unwrap(), "-- original");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn backup_lua_file_noop_when_missing() {
        let tmp = std::env::temp_dir().join("luma_test_backup_noop");
        let _ = fs::remove_dir_all(&tmp);
        let backup_dir = tmp.join("backup");
        let lua_file = tmp.join("nonexistent.lua");

        backup_lua_file(&backup_dir, &lua_file);
        assert!(!backup_dir.exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn package_metadata_roundtrip() {
        let tmp = std::env::temp_dir().join("luma_test_metadata");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let meta_path = tmp.join("metadata.json");
        let meta = PackageMetadata {
            app_id: "12345".into(),
            provider_id: "hubcapdb".into(),
            request_id: Some("req-1-123".into()),
            downloaded_at: "1700000000".into(),
            lua_sha256: Some("abc123".into()),
            manifest_sha256: None,
            lua_files: vec!["12345.lua".into()],
            manifest_files: vec![],
        };
        fs::write(&meta_path, serde_json::to_string(&meta).unwrap()).unwrap();
        let loaded: PackageMetadata =
            serde_json::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
        assert_eq!(loaded.app_id, "12345");
        assert_eq!(loaded.provider_id, "hubcapdb");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn availability_result_serializes() {
        let r = AvailabilityResult {
            available: true,
            file_count: 3,
            total_size: 12345,
            detail: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("available"));
        assert!(json.contains("true"));
    }
}
