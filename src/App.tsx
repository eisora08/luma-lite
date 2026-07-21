import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Sidebar } from './components/Sidebar';
import { DashboardView } from './components/DashboardView';
import { ExtensionsView } from './components/ExtensionsView';
import { SettingsView } from './components/SettingsView';
import type { PluginEntry, SteamRootInfo, ViewId, AppearanceSettings } from './types';
import { DEFAULT_APPEARANCE as DEFAULT_APP } from './types';
import './App.css';

const VIEW_TITLES: Record<ViewId, string> = {
  dashboard: 'Dashboard',
  extensions: 'Extensions',
  settings: 'Settings',
};

function App() {
  const [activeView, setActiveView] = useState<ViewId>('dashboard');
  const [plugins, setPlugins] = useState<PluginEntry[]>([]);
  const [steamRoot, setSteamRoot] = useState<SteamRootInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [sidebarExpanded, setSidebarExpanded] = useState(false);
  const [bridgeRunning, setBridgeRunning] = useState(false);
  const [appearance, setAppearance] = useState<AppearanceSettings>(DEFAULT_APP);

  const win = getCurrentWindow();

  const fetchPlugins = useCallback(async () => {
    try {
      setLoading(true);
      const result = await invoke<{ plugins: PluginEntry[] }>('list_plugins');
      setPlugins(result.plugins);
    } catch (err) {
      console.error('[luma-lite] Failed to load plugins:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchSteamRoot = useCallback(async () => {
    try {
      const info = await invoke<SteamRootInfo>('get_steam_root');
      setSteamRoot(info);
    } catch (err) {
      console.error('[luma-lite] Failed to get Steam root:', err);
    }
  }, []);

  const fetchBridge = useCallback(async () => {
    try {
      const status = await invoke<{ running: boolean; port: number }>('get_bridge_status');
      setBridgeRunning(status.running);
    } catch {
      setBridgeRunning(false);
    }
  }, []);

  const fetchAppearance = useCallback(async () => {
    try {
      const settings = await invoke<AppearanceSettings>('get_appearance_settings');
      setAppearance(settings);
    } catch (err) {
      console.error('[luma-lite] Failed to load appearance:', err);
    }
  }, []);

  useEffect(() => {
    fetchPlugins();
    fetchSteamRoot();
    fetchBridge();
    fetchAppearance();
  }, [fetchPlugins, fetchSteamRoot, fetchBridge, fetchAppearance]);

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', appearance.theme);
    document.documentElement.setAttribute('data-surface', appearance.surfaceStyle);
    document.documentElement.setAttribute('data-density', appearance.density);
    document.documentElement.setAttribute('data-reduce-motion', String(appearance.reduceMotion));
  }, [appearance]);

  const handleAppearanceUpdate = useCallback(async (settings: AppearanceSettings) => {
    setAppearance(settings);
    try {
      await invoke('set_appearance_settings', { settings });
    } catch (err) {
      console.error('[luma-lite] Failed to save appearance:', err);
    }
  }, []);

  const handleToggle = useCallback(async (plugin: PluginEntry): Promise<boolean> => {
    const previousEnabled = plugin.enabled;
    const desiredEnabled = !plugin.enabled;
    console.log(`[luma-lite] handleToggle called for ${plugin.id}: ${previousEnabled} -> ${desiredEnabled}`);

    setPlugins((prev) =>
      prev.map((p) => (p.id === plugin.id ? { ...p, enabled: desiredEnabled } : p))
    );

    try {
      const updated = await invoke<PluginEntry>('toggle_plugin', {
        extensionId: plugin.id,
        enabled: desiredEnabled,
      });

      setPlugins((prev) =>
        prev.map((p) => (p.id === updated.id ? updated : p))
      );
      return true;
    } catch (err) {
      console.error('[luma-lite] Toggle failed:', err);
      setPlugins((prev) =>
        prev.map((p) => (p.id === plugin.id ? { ...p, enabled: previousEnabled } : p))
      );
      return false;
    }
  }, []);

  const handleReload = useCallback(async () => {
    try {
      await invoke('reload_plugins');
      await fetchPlugins();
    } catch (err) {
      console.error('[luma-lite] Reload failed:', err);
    }
  }, [fetchPlugins]);

  return (
    <div className="app-shell">
      <Sidebar
        activeView={activeView}
        onViewChange={setActiveView}
        pluginCount={plugins.length}
        expanded={sidebarExpanded}
        onToggleExpand={() => setSidebarExpanded((v) => !v)}
      />
      <main className="app-main">
        <header className="app-header" data-tauri-drag-region>
          <div className="header-left">
            <span className="app-header-title">LumaForge</span>
            <div className="header-breadcrumb">
              <span className="header-breadcrumb-sep">/</span>
              <span className="header-breadcrumb-current">{VIEW_TITLES[activeView]}</span>
            </div>
          </div>
          <div className="header-spacer" />
          <div className="header-right">
            <div className="header-status" aria-live="polite">
              <span className={`header-status-dot ${bridgeRunning ? '' : 'header-status-dot-offline'}`} />
              <span className="header-status-text">{bridgeRunning ? 'Online' : 'Offline'}</span>
            </div>
            <span className="app-header-version">v0.1.0</span>
            <button
              className="window-btn window-btn-minimize"
              data-tauri-drag-region="noDrag"
              onClick={() => win.minimize()}
              aria-label="Minimize window"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
                <line x1="2" y1="6" x2="10" y2="6" />
              </svg>
            </button>
            <button
              className="window-btn window-btn-close"
              data-tauri-drag-region="noDrag"
              onClick={() => win.hide()}
              aria-label="Close window"
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
                <line x1="3" y1="3" x2="9" y2="9" />
                <line x1="9" y1="3" x2="3" y2="9" />
              </svg>
            </button>
          </div>
        </header>
        <div className="app-view">
          {activeView === 'dashboard' && (
            <DashboardView plugins={plugins} steamRoot={steamRoot} />
          )}
          {activeView === 'extensions' && (
            <ExtensionsView
              plugins={plugins}
              loading={loading}
              onToggle={handleToggle}
              onReload={handleReload}
            />
          )}
          {activeView === 'settings' && (
            <SettingsView
              plugins={plugins}
              steamRoot={steamRoot}
              onSteamRootUpdated={setSteamRoot}
              appearance={appearance}
              onAppearanceUpdated={handleAppearanceUpdate}
            />
          )}
        </div>
      </main>
    </div>
  );
}

export default App;
