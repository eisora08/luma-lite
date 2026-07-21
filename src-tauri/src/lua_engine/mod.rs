use mlua::{Lua, LuaSerdeExt, Table, Value};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub struct LuaEngineConfig {
    pub instruction_limit: u64,
    pub memory_limit: Option<usize>,
}

impl Default for LuaEngineConfig {
    fn default() -> Self {
        Self {
            instruction_limit: 1_000_000,
            memory_limit: Some(10 * 1024 * 1024),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LuaFunctionResult {
    pub success: bool,
    pub value: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LuaExtensionTable {
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub criteria: Option<serde_json::Value>,
    pub has_detect: bool,
    pub has_install: bool,
    pub has_enable: bool,
    pub has_disable: bool,
    pub has_uninstall: bool,
}

pub struct LuaEngine {
    lua: Lua,
    extension_id: String,
    instruction_count: AtomicU64,
}

fn lua_log(level: &str, message: &str) {
    eprintln!("[LUA][{}] {}", level, message);
}

impl LuaEngine {
    pub fn new(config: LuaEngineConfig) -> Result<Self, String> {
        let lua = Lua::new();
        let instruction_count = AtomicU64::new(0);
        let ic = AtomicU64::new(0);
        let il = config.instruction_limit;

        lua.set_hook(
            mlua::HookTriggers::default().every_nth_instruction(1000),
            move |_lua: &Lua, _debug: mlua::Debug| {
                let count = ic.fetch_add(1000, Ordering::Relaxed) + 1000;
                if count > il {
                    ic.store(0, Ordering::Relaxed);
                    return Err(mlua::Error::external("[Lua] Instruction limit exceeded"));
                }
                Ok(mlua::VmState::Continue)
            },
        );

        if let Some(limit) = config.memory_limit {
            let _ = lua.set_memory_limit(limit);
        }

        let sandbox_globals = lua.globals();

        let lumaforge = lua
            .create_table()
            .map_err(|e| format!("Failed to create lumaforge table: {}", e))?;

        Self::register_api_functions(&lua, &lumaforge)?;

        sandbox_globals
            .set("lumaforge", lumaforge)
            .map_err(|e| format!("Failed to set lumaforge global: {}", e))?;

        Ok(Self {
            lua,
            extension_id: String::new(),
            instruction_count,
        })
    }

    fn register_api_functions(lua: &Lua, table: &Table) -> Result<(), String> {
        let file_exists = lua
            .create_function(|_, path: String| Ok(Path::new(&path).exists()))
            .map_err(|e| format!("Failed to create file_exists: {}", e))?;
        table
            .set("file_exists", file_exists)
            .map_err(|e| format!("Failed to register file_exists: {}", e))?;

        let file_status = lua
            .create_function(|lua: &Lua, path: String| {
                let p = Path::new(&path);
                if !p.exists() {
                    return lua.to_value(&serde_json::json!({
                        "exists": false,
                        "size": null,
                        "modifiedAt": null
                    }));
                }
                let meta = match fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => {
                        return lua.to_value(&serde_json::json!({
                            "exists": false,
                            "error": format!("{}", e),
                            "size": null,
                            "modifiedAt": null
                        }));
                    }
                };
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                lua.to_value(&serde_json::json!({
                    "exists": true,
                    "size": meta.len(),
                    "modifiedAt": modified
                }))
            })
            .map_err(|e| format!("Failed to create file_status: {}", e))?;
        table
            .set("file_status", file_status)
            .map_err(|e| format!("Failed to register file_status: {}", e))?;

        let rename_file = lua
            .create_function(|_, (from, to): (String, String)| {
                let from_path = Path::new(&from);
                let to_path = Path::new(&to);
                if !from_path.exists() {
                    return Err(mlua::Error::external(format!(
                        "Source file does not exist: {}",
                        from
                    )));
                }
                if let Some(parent) = to_path.parent() {
                    if !parent.exists() {
                        let _ = fs::create_dir_all(parent);
                    }
                }
                fs::rename(from_path, to_path).map_err(|e| {
                    mlua::Error::external(format!("Rename failed: {} -> {}: {}", from, to, e))
                })
            })
            .map_err(|e| format!("Failed to create rename_file: {}", e))?;
        table
            .set("rename_file", rename_file)
            .map_err(|e| format!("Failed to register rename_file: {}", e))?;

        let copy_file = lua
            .create_function(|_, (from, to): (String, String)| {
                let from_path = Path::new(&from);
                let to_path = Path::new(&to);
                if !from_path.exists() {
                    return Err(mlua::Error::external(format!(
                        "Source file does not exist: {}",
                        from
                    )));
                }
                if let Some(parent) = to_path.parent() {
                    if !parent.exists() {
                        let _ = fs::create_dir_all(parent);
                    }
                }
                fs::copy(from_path, to_path).map_err(|e| {
                    mlua::Error::external(format!("Copy failed: {} -> {}: {}", from, to, e))
                })?;
                Ok(())
            })
            .map_err(|e| format!("Failed to create copy_file: {}", e))?;
        table
            .set("copy_file", copy_file)
            .map_err(|e| format!("Failed to register copy_file: {}", e))?;

        let remove_file = lua
            .create_function(|_, path: String| {
                let p = Path::new(&path);
                if !p.exists() {
                    return Ok(false);
                }
                fs::remove_file(p)
                    .map_err(|e| mlua::Error::external(format!("Failed to remove file: {}", e)))?;
                Ok(true)
            })
            .map_err(|e| format!("Failed to create remove_file: {}", e))?;
        table
            .set("remove_file", remove_file)
            .map_err(|e| format!("Failed to register remove_file: {}", e))?;

        let create_dir = lua
            .create_function(|_, path: String| {
                let p = Path::new(&path);
                if p.exists() {
                    return Ok(false);
                }
                fs::create_dir_all(p).map_err(|e| {
                    mlua::Error::external(format!("Failed to create directory: {}", e))
                })?;
                Ok(true)
            })
            .map_err(|e| format!("Failed to create create_dir: {}", e))?;
        table
            .set("create_dir", create_dir)
            .map_err(|e| format!("Failed to register create_dir: {}", e))?;

        let list_directory = lua
            .create_function(|_, path: String| {
                let dir = Path::new(&path);
                if !dir.exists() || !dir.is_dir() {
                    return Ok(Vec::new() as Vec<String>);
                }
                let names: Vec<String> = match fs::read_dir(dir) {
                    Ok(entries) => entries
                        .flatten()
                        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
                        .collect(),
                    Err(_) => return Ok(Vec::new() as Vec<String>),
                };
                Ok(names)
            })
            .map_err(|e| format!("Failed to create list_directory: {}", e))?;
        table
            .set("list_directory", list_directory)
            .map_err(|e| format!("Failed to register list_directory: {}", e))?;

        let log_fn = lua
            .create_function(|_, (level, message): (String, String)| {
                lua_log(&level, &message);
                Ok(())
            })
            .map_err(|e| format!("Failed to create log function: {}", e))?;
        table
            .set("log", log_fn)
            .map_err(|e| format!("Failed to register log: {}", e))?;

        let get_app_data_dir = lua
            .create_function(|_, _: ()| {
                let app_data = match dirs::data_dir() {
                    Some(d) => d.join("LumaForge"),
                    None => {
                        return Err(mlua::Error::external(
                            "Could not determine app data directory",
                        ))
                    }
                };
                Ok(app_data.to_string_lossy().to_string())
            })
            .map_err(|e| format!("Failed to create get_app_data_dir: {}", e))?;
        table
            .set("get_app_data_dir", get_app_data_dir)
            .map_err(|e| format!("Failed to register get_app_data_dir: {}", e))?;

        let get_extension_dir = lua
            .create_function(|_, extension_id: String| {
                let app_data = match dirs::data_dir() {
                    Some(d) => d.join("LumaForge"),
                    None => {
                        return Err(mlua::Error::external(
                            "Could not determine app data directory",
                        ))
                    }
                };
                let ext_dir = app_data.join("extensions").join(&extension_id);
                let _ = fs::create_dir_all(&ext_dir);
                Ok(ext_dir.to_string_lossy().to_string())
            })
            .map_err(|e| format!("Failed to create get_extension_dir: {}", e))?;
        table
            .set("get_extension_dir", get_extension_dir)
            .map_err(|e| format!("Failed to register get_extension_dir: {}", e))?;

        let find_largest_exe = lua
            .create_function(|_, (dir, exclude): (String, Vec<String>)| {
                let dir_path = Path::new(&dir);
                if !dir_path.exists() || !dir_path.is_dir() {
                    return Ok(None::<String>);
                }
                let exclude_lower: Vec<String> = exclude.iter().map(|s| s.to_lowercase()).collect();
                let entries = match fs::read_dir(dir_path) {
                    Ok(e) => e,
                    Err(_) => return Ok(None::<String>),
                };
                let mut largest: Option<(String, u64)> = None;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) != Some("exe") {
                        continue;
                    }
                    let name = entry.file_name().to_string_lossy().to_string();
                    let name_lower = name.to_lowercase();
                    if exclude_lower.contains(&name_lower) {
                        continue;
                    }
                    if let Ok(meta) = path.metadata() {
                        if meta.is_file() {
                            let size = meta.len();
                            if largest.as_ref().is_none_or(|(_, s)| size > *s) {
                                largest = Some((name, size));
                            }
                        }
                    }
                }
                Ok(largest.map(|(n, _)| n))
            })
            .map_err(|e| format!("Failed to create find_largest_exe: {}", e))?;
        table
            .set("find_largest_exe", find_largest_exe)
            .map_err(|e| format!("Failed to register find_largest_exe: {}", e))?;

        let write_text_file = lua
            .create_function(|_, (path, content): (String, String)| {
                let p = Path::new(&path);
                if let Some(parent) = p.parent() {
                    if !parent.exists() {
                        fs::create_dir_all(parent).map_err(|e| {
                            mlua::Error::external(format!(
                                "Failed to create parent directory: {}",
                                e
                            ))
                        })?;
                    }
                }
                fs::write(p, &content)
                    .map_err(|e| mlua::Error::external(format!("Failed to write file: {}", e)))
            })
            .map_err(|e| format!("Failed to create write_text_file: {}", e))?;
        table
            .set("write_text_file", write_text_file)
            .map_err(|e| format!("Failed to register write_text_file: {}", e))?;

        let fetch_url = lua
            .create_function(|_lua: &Lua, url: String| {
                let client = reqwest::blocking::Client::builder()
                    .user_agent("LumaForge-Lite/0.1.0")
                    .timeout(std::time::Duration::from_secs(30))
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()
                    .map_err(|e| mlua::Error::external(format!("HTTP client error: {}", e)))?;
                let response = client
                    .get(&url)
                    .send()
                    .map_err(|e| mlua::Error::external(format!("HTTP request failed: {}", e)))?;
                if !response.status().is_success() {
                    return Err(mlua::Error::external(format!(
                        "HTTP {} for {}",
                        response.status(),
                        url
                    )));
                }
                let text = response.text().map_err(|e| {
                    mlua::Error::external(format!("Failed to read response: {}", e))
                })?;
                Ok(text)
            })
            .map_err(|e| format!("Failed to create fetch_url: {}", e))?;
        table
            .set("fetch_url", fetch_url)
            .map_err(|e| format!("Failed to register fetch_url: {}", e))?;

        let download_file = lua
            .create_function(|_, (url, target_path): (String, String)| {
                let client = reqwest::blocking::Client::builder()
                    .user_agent("LumaForge-Lite/0.1.0")
                    .timeout(std::time::Duration::from_secs(120))
                    .connect_timeout(std::time::Duration::from_secs(15))
                    .redirect(reqwest::redirect::Policy::limited(5))
                    .build()
                    .map_err(|e| mlua::Error::external(format!("HTTP client error: {}", e)))?;
                let mut response = client
                    .get(&url)
                    .send()
                    .map_err(|e| mlua::Error::external(format!("Download failed: {}", e)))?;
                if !response.status().is_success() {
                    return Err(mlua::Error::external(format!(
                        "Download failed with status: {}",
                        response.status()
                    )));
                }
                let target = Path::new(&target_path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| {
                        mlua::Error::external(format!("Failed to create target directory: {}", e))
                    })?;
                }
                let mut file = fs::File::create(target).map_err(|e| {
                    mlua::Error::external(format!("Failed to create target file: {}", e))
                })?;
                std::io::copy(&mut response, &mut file)
                    .map_err(|e| mlua::Error::external(format!("Failed to write file: {}", e)))?;
                lua_log("INFO", &format!("downloaded: {} -> {}", url, target_path));
                Ok(())
            })
            .map_err(|e| format!("Failed to create download_file: {}", e))?;
        table
            .set("download_file", download_file)
            .map_err(|e| format!("Failed to register download_file: {}", e))?;

        let extract_zip = lua
            .create_function(|_lua: &Lua, (zip_path, target_dir, expected_files): (String, String, Vec<String>)| {
                use zip::read::ZipArchive;
                let zip_file = fs::File::open(&zip_path)
                    .map_err(|e| mlua::Error::external(format!("Failed to open zip: {}", e)))?;
                let mut archive = ZipArchive::new(zip_file)
                    .map_err(|e| mlua::Error::external(format!("Failed to read zip archive: {}", e)))?;
                let target = Path::new(&target_dir);
                fs::create_dir_all(target)
                    .map_err(|e| mlua::Error::external(format!("Failed to create extract directory: {}", e)))?;
                let expected_lower: Vec<String> = expected_files
                    .iter().map(|f| f.to_lowercase()).collect();
                let mut extracted: Vec<String> = Vec::new();
                for i in 0..archive.len() {
                    let mut entry = archive.by_index(i)
                        .map_err(|e| mlua::Error::external(format!("Failed to read zip entry: {}", e)))?;
                    let entry_name = entry.name().to_string();
                    let entry_lower = entry_name.to_lowercase();
                    let matches = expected_lower.iter().any(|expected| {
                        entry_lower == *expected
                            || entry_lower.ends_with(&format!("/{}", expected))
                            || entry_lower.split('/').next_back().map(|n| n == expected.as_str()).unwrap_or(false)
                    });
                    if !matches { continue; }
                    let file_name = entry_name.split('/').next_back().unwrap_or(&entry_name);
                    let out_path = target.join(file_name);
                    if entry.is_dir() {
                        fs::create_dir_all(&out_path)
                            .map_err(|e| mlua::Error::external(format!("Failed to create directory: {}", e)))?;
                    } else {
                        if let Some(parent) = out_path.parent() {
                            fs::create_dir_all(parent)
                                .map_err(|e| mlua::Error::external(format!("Failed to create parent: {}", e)))?;
                        }
                        let mut out_file = fs::File::create(&out_path)
                            .map_err(|e| mlua::Error::external(format!("Failed to create file: {}", e)))?;
                        std::io::copy(&mut entry, &mut out_file)
                            .map_err(|e| mlua::Error::external(format!("Failed to extract file: {}", e)))?;
                        extracted.push(file_name.to_string());
                    }
                }
                Ok(extracted)
            })
            .map_err(|e| format!("Failed to create extract_zip: {}", e))?;
        table
            .set("extract_zip", extract_zip)
            .map_err(|e| format!("Failed to register extract_zip: {}", e))?;

        let run_process = lua
            .create_function(|lua: &Lua, (exe_path, args): (String, Vec<String>)| {
                use std::process::Command;
                let exe = Path::new(&exe_path);
                if !exe.exists() {
                    return Err(mlua::Error::external(format!(
                        "Executable not found: {}",
                        exe_path
                    )));
                }
                let output = Command::new(&exe_path)
                    .args(&args)
                    .output()
                    .map_err(|e| mlua::Error::external(format!("Failed to run process: {}", e)))?;
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                let exit_code = output.status.code().unwrap_or(-1);
                lua.to_value(&serde_json::json!({
                    "success": output.status.success(),
                    "exitCode": exit_code,
                    "stdout": stdout,
                    "stderr": stderr
                }))
            })
            .map_err(|e| format!("Failed to create run_process: {}", e))?;
        table
            .set("run_process", run_process)
            .map_err(|e| format!("Failed to register run_process: {}", e))?;

        Ok(())
    }

    pub fn load_and_evaluate(
        &mut self,
        extension_id: &str,
        script: &str,
    ) -> Result<LuaExtensionTable, String> {
        self.instruction_count.store(0, Ordering::Relaxed);
        self.lua
            .load(script)
            .exec()
            .map_err(|e| format!("Failed to load extension script: {}", e))?;

        let globals = self.lua.globals();
        let ext_value: Value = globals
            .get("extension")
            .map_err(|e| format!("Failed to get 'extension' global: {}", e))?;

        match ext_value {
            Value::Table(ref tbl) => {
                let id: Option<String> = tbl.get("id").ok();
                let name: Option<String> = tbl.get("name").ok();
                let version: Option<String> = tbl.get("version").ok();
                let description: Option<String> = tbl.get("description").ok();
                let criteria: Option<serde_json::Value> = tbl
                    .get::<mlua::Value>("criteria")
                    .ok()
                    .and_then(|v| self.lua.from_value(v).ok());
                let ext = LuaExtensionTable {
                    id,
                    name,
                    version,
                    description,
                    criteria,
                    has_detect: tbl.contains_key("detect").unwrap_or(false),
                    has_install: tbl.contains_key("install").unwrap_or(false),
                    has_enable: tbl.contains_key("enable").unwrap_or(false),
                    has_disable: tbl.contains_key("disable").unwrap_or(false),
                    has_uninstall: tbl.contains_key("uninstall").unwrap_or(false),
                };
                self.extension_id = extension_id.to_string();
                Ok(ext)
            }
            _ => Err("Extension script must define a global 'extension' table".to_string()),
        }
    }

    #[allow(dead_code)]
    pub fn has_function(&self, name: &str) -> bool {
        let globals = match self.lua.globals().get::<Table>("extension") {
            Ok(t) => t,
            Err(_) => return false,
        };
        globals.get::<mlua::Function>(name).is_ok()
    }

    pub fn call_function(
        &self,
        name: &str,
        install_dir: &str,
    ) -> Result<LuaFunctionResult, String> {
        self.instruction_count.store(0, Ordering::Relaxed);

        let globals = self.lua.globals();
        let ext_table: Table = globals
            .get("extension")
            .map_err(|e| format!("Failed to get extension table: {}", e))?;

        let func: mlua::Function = ext_table
            .get(name)
            .map_err(|_| format!("Extension does not define '{}' function", name))?;

        let result: Value = func
            .call::<Value>(install_dir)
            .map_err(|e| format!("Extension function '{}' failed: {}", name, e))?;

        let json_value: serde_json::Value = match self.lua.from_value(result) {
            Ok(v) => v,
            Err(_) => serde_json::Value::Null,
        };

        Ok(LuaFunctionResult {
            success: true,
            value: Some(json_value),
            error: None,
        })
    }
}
