mod commands;
mod config;
mod lua_engine;

/// Boot-time re-injection: for each enabled extension with CEF injection,
/// read the inject script from the resolved plugin path and inject into
/// matching Steam browser tabs.
fn re_inject_enabled_extensions(plugins: &[commands::plugins::PluginEntry]) {
    for plugin in plugins {
        if !plugin.enabled {
            continue;
        }
        if plugin.cef_injection != Some(true) {
            continue;
        }
        let inject_script = match &plugin.inject_script {
            Some(s) => s.clone(),
            None => continue,
        };
        let target_url = plugin
            .target_url
            .clone()
            .unwrap_or_else(|| "store.steampowered.com".into());

        // Use the canonical AppData resolver
        let script_path = match config::resolve_inject_script(&plugin.id, &inject_script) {
            Some(p) => p,
            None => {
                eprintln!(
                    "[BOOT] Inject script not found for '{}' in AppData",
                    plugin.id
                );
                continue;
            }
        };

        eprintln!("[PLUGIN_RUNTIME] Plugin ID: {}", plugin.id);
        eprintln!(
            "[PLUGIN_RUNTIME] Inject script from manifest: {}",
            inject_script
        );
        eprintln!(
            "[PLUGIN_RUNTIME] Resolved script path: {}",
            script_path.display()
        );
        eprintln!(
            "[PLUGIN_RUNTIME] Script size: {}",
            std::fs::metadata(&script_path)
                .map(|m| m.len())
                .unwrap_or(0)
        );

        match std::fs::read_to_string(&script_path) {
            Ok(js_code) => {
                // Extract version marker from script content
                let version_marker = js_code
                    .lines()
                    .find(|l| l.contains("LUMA_INJECT_VERSION"))
                    .and_then(|l| l.split('\'').nth(1).or_else(|| l.split('"').nth(1)))
                    .unwrap_or("(not found)");
                eprintln!("[PLUGIN_RUNTIME] Script version marker: {}", version_marker);

                eprintln!(
                    "[BOOT] Re-injecting '{}' into tabs matching '{}' (script: {})",
                    plugin.id,
                    target_url,
                    script_path.display()
                );
                let result =
                    commands::steam_inject::inject_code_into_tabs(&target_url, &js_code, &[]);
                commands::steam_inject::track_injection(
                    &plugin.id,
                    &target_url,
                    &result.injected_target_ids,
                    &result.injected_tab_urls,
                    &std::iter::repeat(None)
                        .take(result.injected_target_ids.len())
                        .collect::<Vec<_>>(),
                    &std::iter::repeat(None)
                        .take(result.injected_target_ids.len())
                        .collect::<Vec<_>>(),
                );
                if result.success {
                    eprintln!(
                        "[BOOT] Injected into {} tab(s) for '{}'",
                        result.injected_tab_urls.len(),
                        plugin.id
                    );
                    commands::steam_inject::start_target_monitor(plugin.id.clone(), target_url);
                } else if let Some(ref err) = result.error {
                    eprintln!("[BOOT] Injection for '{}': {err}", plugin.id);
                }
            }
            Err(e) => {
                eprintln!(
                    "[BOOT] Failed to read inject script {}: {e}",
                    script_path.display()
                );
            }
        }
    }
}

/// Seed builtin plugins from the repository source into the AppData runtime directory.
/// Idempotent: only copies when the runtime plugin directory is missing.
fn seed_builtin_plugins(runtime_plugins_dir: &std::path::Path) {
    // Locate the builtin source: plugins/builtin/ relative to workspace root.
    // CARGO_MANIFEST_DIR is compile-time (src-tauri/), so we pop the last
    // component to reach the repo root, then join plugins/builtin.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap_or(manifest_dir);
    let builtin_source = workspace_root.join("plugins").join("builtin");

    if !builtin_source.is_dir() {
        eprintln!(
            "[PLUGIN_MIGRATION] No builtin plugin source found at {}. Skipping seed.",
            builtin_source.display()
        );
        return;
    }

    let dir_entries = match std::fs::read_dir(&builtin_source) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
                "[PLUGIN_MIGRATION] Failed to read builtin source {}: {e}",
                builtin_source.display()
            );
            return;
        }
    };

    for entry in dir_entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }

        let plugin_id = entry.file_name().to_string_lossy().to_string();
        let runtime_dir = runtime_plugins_dir.join(&plugin_id);

        if runtime_dir.exists() {
            eprintln!("[PLUGIN_MIGRATION] Runtime plugin already exists in AppData: {plugin_id}");
            continue;
        }

        // Copy the builtin plugin to AppData
        if let Err(e) = copy_dir_recursive(&entry.path(), &runtime_dir) {
            eprintln!("[PLUGIN_MIGRATION] Failed to seed builtin plugin {plugin_id}: {e}");
            continue;
        }

        eprintln!("[PLUGIN_MIGRATION] Seeded builtin plugin: {plugin_id}");
    }
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

pub fn run() {
    // Forensic identity logs
    if let Ok(exe) = std::env::current_exe() {
        eprintln!("[RUNTIME_ID] Executable path: {}", exe.display());
        if let Some(parent) = exe.parent() {
            eprintln!("[RUNTIME_ID] Executable directory: {}", parent.display());
        }
    }
    eprintln!("[RUNTIME_ID] Process ID: {}", std::process::id());
    eprintln!(
        "[RUNTIME_ID] Build profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    eprintln!("[RUNTIME_ID] Package name: {}", env!("CARGO_PKG_NAME"));
    eprintln!(
        "[RUNTIME_ID] Package version: {}",
        env!("CARGO_PKG_VERSION")
    );
    if let Ok(cwd) = std::env::current_dir() {
        eprintln!("[RUNTIME_ID] Current working directory: {}", cwd.display());
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let handle = app.handle().clone();

            // Load config and detect Steam root
            let _ = config::load_config();
            match config::resolve_steam_root() {
                Some(p) => eprintln!("[BOOT] Steam root: {}", p.display()),
                None => eprintln!("[BOOT] Steam root not found — extensions may not work"),
            }

            // Seed builtin plugins into AppData, then scan from AppData
            let plugins_dir = config::resolve_plugins_dir();
            eprintln!(
                "[PLUGIN_RUNTIME] Plugins directory: {}",
                plugins_dir.display()
            );
            seed_builtin_plugins(&plugins_dir);
            let plugins = match commands::plugins::do_scan_plugins(None) {
                Ok(p) => {
                    if !p.is_empty() {
                        eprintln!(
                            "[BOOT] Scanned {} plugins from {}",
                            p.len(),
                            plugins_dir.display()
                        );
                    }
                    p
                }
                Err(e) => {
                    eprintln!("[BOOT] Plugin scan failed: {e}");
                    vec![]
                }
            };

            // Start the Steam CEF HTTP bridge
            commands::steam_bridge::start_steam_bridge(handle.clone());

            // Wait for the bridge to be ready before reinjecting
            eprintln!("[BOOT] Waiting for Steam bridge readiness");
            let bridge_ready = commands::steam_bridge::wait_until_bridge_ready(5000);
            if bridge_ready {
                eprintln!("[BOOT] Steam bridge ready; injecting enabled extensions");
                re_inject_enabled_extensions(&plugins);
            } else {
                eprintln!("[BOOT] Bridge was not ready; deferring enabled extension injection");
            }

            // Initialize system tray
            commands::tray::init_system_tray(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // App config
            config::get_steam_root,
            config::set_steam_root,
            config::get_appearance_settings,
            config::set_appearance_settings,
            config::get_providers,
            config::set_providers,
            config::get_downloads_config,
            config::get_multi_provider_fallback,
            config::set_multi_provider_fallback,
            // Steam bridge
            commands::steam_bridge::get_bridge_status,
            // CDP injection
            commands::steam_inject::inject_to_steam_tab,
            commands::steam_inject::inject_plugin_by_id,
            commands::steam_inject::get_injection_status,
            // Extension file operations
            commands::extension::extension_file_exists,
            commands::extension::extension_file_status,
            commands::extension::extension_rename_file,
            commands::extension::extension_copy_file,
            commands::extension::extension_remove_file,
            commands::extension::extension_create_dir,
            commands::extension::extension_list_directory,
            commands::extension::extension_delete_directory,
            commands::extension::extension_open_url,
            commands::extension::extension_show_in_folder,
            commands::extension::extension_write_config,
            commands::extension::extension_read_config,
            commands::extension::extension_get_plugins_dir,
            commands::extension::extension_open_plugins_folder,
            // Lua lifecycle
            commands::extension_lifecycle::load_extension,
            commands::extension_lifecycle::call_extension_detect,
            commands::extension_lifecycle::call_extension_install,
            commands::extension_lifecycle::call_extension_enable,
            commands::extension_lifecycle::call_extension_disable,
            commands::extension_lifecycle::call_extension_uninstall,
            commands::extension_lifecycle::read_extension_text_file,
            // Plugin management
            commands::plugins::list_plugins,
            commands::plugins::scan_plugins,
            commands::plugins::toggle_plugin,
            commands::plugins::reload_plugins,
        ])
        .run(tauri::generate_context!())
        .expect("error while running luma-lite");
}
