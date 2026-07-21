use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const HUBCAP_BASE_URL: &str = "https://hubcapmanifest.com/api/v1";
const HUBCAP_TIMEOUT_SECS: u64 = 30;

// ---------------------------------------------------------------------------
// API Key storage (in-memory, persisted via AppConfig)
// ---------------------------------------------------------------------------

static API_KEY: Mutex<Option<String>> = Mutex::new(None);

pub fn get_api_key() -> Option<String> {
    let guard = API_KEY.lock().unwrap();
    guard.clone()
}

fn build_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(HUBCAP_TIMEOUT_SECS))
        .danger_accept_invalid_certs(false)
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

// ---------------------------------------------------------------------------
// Download & extraction
// ---------------------------------------------------------------------------

/// Download a package ZIP from Hubcap and extract to the Steam directories.
/// - `.lua` files → `{steam_root}/config/lua/`
/// - `.manifest` files → `{steam_root}/depotcache/`
///
/// Returns a summary of what was extracted.
pub fn download_and_extract(app_id: &str, steam_root: &Path) -> Result<ExtractionResult, String> {
    let api_key = get_api_key().ok_or_else(|| "No Hubcap API key configured.".to_string())?;

    let url = format!("{HUBCAP_BASE_URL}/download/{app_id}");
    let client = build_client();

    eprintln!("[HUBCAP] Downloading package for app_id={app_id} from {url}");

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

    // Read entire response into memory
    let bytes = resp
        .bytes()
        .map_err(|e| format!("Failed to read download response: {e}"))?;

    eprintln!("[HUBCAP] Downloaded {} bytes, extracting...", bytes.len());

    // Extract ZIP from memory
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("Failed to open ZIP archive: {e}"))?;

    let lua_dir = steam_root.join("config").join("lua");
    let depot_dir = steam_root.join("depotcache");

    // Ensure directories exist
    fs::create_dir_all(&lua_dir)
        .map_err(|e| format!("Failed to create config/lua directory: {e}"))?;
    fs::create_dir_all(&depot_dir)
        .map_err(|e| format!("Failed to create depotcache directory: {e}"))?;

    let mut result = ExtractionResult {
        lua_files: Vec::new(),
        manifest_files: Vec::new(),
        errors: Vec::new(),
    };

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

        if lower_name.ends_with(".lua") {
            // Extract .lua file to config/lua/
            let file_name = Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = lua_dir.join(&file_name);

            match extract_entry(&mut entry, &dest) {
                Ok(_) => {
                    eprintln!("[HUBCAP] Extracted LUA: {}", dest.display());
                    result.lua_files.push(dest.to_string_lossy().to_string());
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to extract {entry_name}: {e}"));
                }
            }
        } else if lower_name.ends_with(".manifest") {
            // Extract .manifest file to depotcache/
            let file_name = Path::new(&entry_name)
                .file_name()
                .map(|f| f.to_os_string())
                .unwrap_or_default();
            let dest = depot_dir.join(&file_name);

            match extract_entry(&mut entry, &dest) {
                Ok(_) => {
                    eprintln!("[HUBCAP] Extracted MANIFEST: {}", dest.display());
                    result
                        .manifest_files
                        .push(dest.to_string_lossy().to_string());
                }
                Err(e) => {
                    result
                        .errors
                        .push(format!("Failed to extract {entry_name}: {e}"));
                }
            }
        }
        // Ignore other file types
    }

    eprintln!(
        "[HUBCAP] Extraction complete: {} LUA, {} manifest, {} errors",
        result.lua_files.len(),
        result.manifest_files.len(),
        result.errors.len()
    );

    Ok(result)
}

fn extract_entry(reader: &mut impl Read, dest: &Path) -> Result<(), String> {
    let mut file =
        fs::File::create(dest).map_err(|e| format!("Failed to create {}: {e}", dest.display()))?;
    std::io::copy(reader, &mut file)
        .map_err(|e| format!("Failed to write {}: {e}", dest.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub lua_files: Vec<String>,
    pub manifest_files: Vec<String>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Local status check
// ---------------------------------------------------------------------------

/// Check if a lua file exists for the given app_id in the Steam config/lua/ directory.
pub fn check_local_status(app_id: &str, steam_root: &Path) -> bool {
    let lua_path = steam_root
        .join("config")
        .join("lua")
        .join(format!("{app_id}.lua"));
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

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn extraction_result_serializes() {
        let r = ExtractionResult {
            lua_files: vec!["test.lua".into()],
            manifest_files: vec!["test.manifest".into()],
            errors: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("test.lua"));
        assert!(json.contains("test.manifest"));
    }
}
