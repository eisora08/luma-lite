use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// File existence & metadata
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn extension_file_exists(path: String) -> bool {
    Path::new(&path).exists()
}

#[tauri::command]
pub fn extension_file_status(path: String) -> Result<serde_json::Value, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Ok(serde_json::json!({
            "exists": false,
            "isFile": false,
            "isDir": false,
            "size": null,
            "modified": null,
        }));
    }
    let meta = fs::metadata(p).map_err(|e| format!("Failed to read metadata for {path}: {e}"))?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Ok(serde_json::json!({
        "exists": true,
        "isFile": meta.is_file(),
        "isDir": meta.is_dir(),
        "size": meta.len(),
        "modified": modified,
    }))
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn extension_rename_file(from: String, to: String) -> Result<(), String> {
    let from_path = Path::new(&from);
    let to_path = Path::new(&to);
    if !from_path.exists() {
        return Err(format!("Source does not exist: {from}"));
    }
    if let Some(parent) = to_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
        }
    }
    fs::rename(from_path, to_path).map_err(|e| format!("Rename failed {from} -> {to}: {e}"))
}

#[tauri::command]
pub fn extension_copy_file(from: String, to: String) -> Result<(), String> {
    let from_path = Path::new(&from);
    let to_path = Path::new(&to);
    if !from_path.exists() {
        return Err(format!("Source does not exist: {from}"));
    }
    if let Some(parent) = to_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
        }
    }
    fs::copy(from_path, to_path).map_err(|e| format!("Copy failed {from} -> {to}: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn extension_remove_file(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Ok(());
    }
    fs::remove_file(p).map_err(|e| format!("Failed to remove {path}: {e}"))
}

#[tauri::command]
pub fn extension_create_dir(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Ok(());
    }
    fs::create_dir_all(p).map_err(|e| format!("Failed to create directory {path}: {e}"))
}

#[tauri::command]
pub fn extension_list_directory(path: String) -> Result<Vec<String>, String> {
    let dir = Path::new(&path);
    if !dir.exists() || !dir.is_dir() {
        return Ok(vec![]);
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read directory {path}: {e}"))?;
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    Ok(names)
}

#[tauri::command]
pub fn extension_delete_directory(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Ok(());
    }
    fs::remove_dir_all(p).map_err(|e| format!("Failed to delete directory {path}: {e}"))
}

// ---------------------------------------------------------------------------
// URL / folder helpers
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn extension_open_url(url: String) -> Result<(), String> {
    open::that(&url).map_err(|e| format!("Failed to open URL {url}: {e}"))
}

#[tauri::command]
pub fn extension_show_in_folder(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    let dir = if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .ok_or_else(|| format!("Cannot determine parent directory for {path}"))?
            .to_path_buf()
    };
    open::that(&dir).map_err(|e| format!("Failed to open folder {}: {e}", dir.display()))
}

// ---------------------------------------------------------------------------
// Config file helpers
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn extension_write_config(path: String, content: String) -> Result<(), String> {
    let p = Path::new(&path);
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directory: {e}"))?;
        }
    }
    fs::write(p, &content).map_err(|e| format!("Failed to write config {path}: {e}"))
}

#[tauri::command]
pub fn extension_read_config(path: String) -> Result<Option<String>, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Ok(None);
    }
    let content =
        fs::read_to_string(p).map_err(|e| format!("Failed to read config {path}: {e}"))?;
    Ok(Some(content))
}

// ---------------------------------------------------------------------------
// Plugins directory helpers
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn extension_get_plugins_dir() -> Result<String, String> {
    Ok(crate::config::resolve_plugins_dir()
        .to_string_lossy()
        .to_string())
}

#[tauri::command]
pub fn extension_open_plugins_folder() -> Result<(), String> {
    let dir = crate::config::resolve_plugins_dir();
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("Failed to create plugins directory: {e}"))?;
    }
    open::that(&dir).map_err(|e| format!("Failed to open plugins folder: {e}"))
}
