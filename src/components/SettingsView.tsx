import { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { PluginEntry, SteamRootInfo, SettingsCategoryId, SettingsSubView, AppearanceSettings, ProviderConfig, ProviderDef } from '../types';
import { AppearanceView } from './AppearanceView';

interface SettingsViewProps {
  plugins: PluginEntry[];
  steamRoot: SteamRootInfo | null;
  onSteamRootUpdated: (info: SteamRootInfo) => void;
  appearance: AppearanceSettings;
  onAppearanceUpdated: (settings: AppearanceSettings) => void;
}

const CATEGORIES: { id: SettingsCategoryId; label: string; description: string; icon: JSX.Element }[] = [
  {
    id: 'general',
    label: 'General',
    description: 'Application preferences and information',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="10" cy="10" r="2.5" />
        <path d="M17.4 10c0-.5-.3-1-.8-1.2l-1-.4c-.1-.4-.3-.8-.5-1.1l.3-1c.3-.5.2-1.1-.2-1.5l-.7-.7c-.4-.4-1-.5-1.5-.2l-1 .3c-.4-.2-.8-.4-1.1-.5l-.4-1c-.3-.5-.8-.8-1.2-.8s-1 .3-1.2.8l-.4 1c-.4.1-.8.3-1.1.5l-1-.3c-.5-.3-1.1-.2-1.5.2l-.7.7c-.4.4-.5 1-.2 1.5l.3 1c-.2.4-.4.8-.5 1.1l-1 .4c-.5.3-.8.7-.8 1.2s.3 1 .8 1.2l1 .4c.1.4.3.8.5 1.1l-.3 1c-.3.5-.2 1.1.2 1.5l.7.7c.4.4 1 .5 1.5.2l1-.3c.4.2.8.4 1.1.5l.4 1c.3.5.8.8 1.2.8s1-.3 1.2-.8l.4-1c.4-.1.8-.3 1.1-.5l1 .3c.5.3 1.1.2 1.5-.2l.7-.7c.4-.4.5-1 .2-1.5l-.3-1c.2-.4.4-.8.5-1.1l1-.4c.5-.3.8-.7.8-1.2z" />
      </svg>
    ),
  },
  {
    id: 'appearance',
    label: 'Appearance',
    description: 'Themes, surfaces, density, and visual preferences',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="10" cy="7" r="4" />
        <path d="M10 11v3M7 14h6" />
        <circle cx="10" cy="7" r="1.5" />
      </svg>
    ),
  },
  {
    id: 'extensions',
    label: 'Extensions',
    description: 'Manage installed extensions and extension permissions',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <path d="M10 2v4M10 14v4M2 10h4M14 10h4" />
        <circle cx="10" cy="10" r="2.5" />
      </svg>
    ),
  },
  {
    id: 'integrations',
    label: 'Integrations',
    description: 'Configure Steam and external services',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M3 7l7-4 7 4v6l-7 4-7-4V7z" />
        <path d="M3 7l7 4 7-4M10 11v7" />
      </svg>
    ),
  },
  {
    id: 'downloads',
    label: 'Downloads / Packages',
    description: 'Manage download providers and package sources',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M10 3v8M6.5 9.5L10 13l3.5-3.5" />
        <path d="M3 14v1.5a1 1 0 001 1h12a1 1 0 001-1V14" />
      </svg>
    ),
  },
  {
    id: 'advanced',
    label: 'Advanced',
    description: 'Diagnostics and advanced application behavior',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <polyline points="3 6 5 6 17 6" />
        <path d="M6 6V4a2 2 0 012-2h4a2 2 0 012 2v2" />
        <path d="M4 6l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12" />
      </svg>
    ),
  },
  {
    id: 'about',
    label: 'About',
    description: 'Version, licenses, and application information',
    icon: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="10" cy="10" r="7" />
        <line x1="10" y1="9" x2="10" y2="14" />
        <circle cx="10" cy="6.5" r="0.5" fill="currentColor" />
      </svg>
    ),
  },
];

function ChevronRight() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="6 4 10 8 6 12" />
    </svg>
  );
}

function SettingsHome({ onSelect, plugins, steamRoot }: {
  onSelect: (id: SettingsCategoryId) => void;
  plugins: PluginEntry[];
  steamRoot: SteamRootInfo | null;
}) {
  return (
    <div className="view-content">
      <div className="settings-home-header">
        <h2 className="settings-home-title">Settings</h2>
        <p className="settings-home-desc">Configure LumaForge Lite to match your workflow.</p>
      </div>

      <div className="settings-home-list" role="list">
        {CATEGORIES.map((cat) => {
          let badge: string | undefined;
          if (cat.id === 'extensions') {
            badge = `${plugins.length}`;
          } else if (cat.id === 'integrations') {
            badge = steamRoot?.resolvedPath ? 'Active' : undefined;
          }
          return (
            <button
              key={cat.id}
              className="settings-home-row"
              onClick={() => onSelect(cat.id)}
              role="listitem"
            >
              <span className="settings-home-row-icon">{cat.icon}</span>
              <div className="settings-home-row-text">
                <span className="settings-home-row-label">{cat.label}</span>
                <span className="settings-home-row-desc">{cat.description}</span>
              </div>
              {badge && <span className="settings-home-row-badge">{badge}</span>}
              <span className="settings-home-row-chevron"><ChevronRight /></span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function GeneralScreen({ plugins, steamRoot }: { plugins: PluginEntry[]; steamRoot: SteamRootInfo | null }) {
  const enabledCount = plugins.filter((p) => p.enabled).length;
  return (
    <div className="view-content">
      <SettingsBreadcrumb category="General" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">General</h2>
        <p className="settings-category-desc">Application preferences and common settings.</p>
      </div>
      <div className="settings-sections">
        <div className="settings-group">
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Plugins Directory</span>
              <span className="settings-row-desc">Location where extensions are loaded from</span>
            </div>
            <div className="settings-row-right">
              <span className="settings-status-badge">Auto-detected</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Extensions Loaded</span>
              <span className="settings-row-desc">{enabledCount} enabled of {plugins.length} total</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Steam Bridge</span>
              <span className="settings-row-desc">CEF communication bridge on port 21775</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function IntegrationsScreen({ steamRoot, onSteamRootUpdated }: { steamRoot: SteamRootInfo | null; onSteamRootUpdated: (info: SteamRootInfo) => void }) {
  const [editPath, setEditPath] = useState('');
  const [saving, setSaving] = useState(false);

  const handleSave = useCallback(async () => {
    if (!editPath.trim()) return;
    setSaving(true);
    try {
      const result = await invoke<SteamRootInfo>('set_steam_root', { path: editPath.trim() });
      onSteamRootUpdated(result);
      setEditPath('');
    } catch (err) {
      console.error('[luma-lite] Failed to save Steam path:', err);
    } finally {
      setSaving(false);
    }
  }, [editPath, onSteamRootUpdated]);

  const handleClear = useCallback(async () => {
    setSaving(true);
    try {
      const result = await invoke<SteamRootInfo>('set_steam_root', { path: null });
      onSteamRootUpdated(result);
    } catch (err) {
      console.error('[luma-lite] Failed to clear Steam path:', err);
    } finally {
      setSaving(false);
    }
  }, [onSteamRootUpdated]);

  const handleAutoDetect = useCallback(async () => {
    setSaving(true);
    try {
      const result = await invoke<SteamRootInfo>('set_steam_root', { path: null });
      onSteamRootUpdated(result);
    } catch (err) {
      console.error('[luma-lite] Failed to auto-detect:', err);
    } finally {
      setSaving(false);
    }
  }, [onSteamRootUpdated]);

  return (
    <div className="view-content">
      <SettingsBreadcrumb category="Integrations" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">Integrations</h2>
        <p className="settings-category-desc">Configure Steam and external services.</p>
      </div>
      <div className="settings-sections">
        <div className="settings-group">
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Steam Root Path</span>
              <span className="settings-row-desc" title={steamRoot?.resolvedPath ?? 'Not found'}>
                {steamRoot?.resolvedPath ?? 'Not detected'}
              </span>
            </div>
            <div className="settings-row-right">
              {steamRoot?.isCustom && <span className="settings-status-badge">custom</span>}
              {!steamRoot?.isCustom && steamRoot?.resolvedPath && (
                <span className="settings-status-badge settings-status-badge-auto">auto-detected</span>
              )}
            </div>
          </div>
        </div>

        <div className="settings-group-title">Change Path</div>
        <div className="settings-group">
          <div className="settings-row settings-row-stacked">
            <div className="settings-row-left">
              <span className="settings-row-label">Custom Steam Root</span>
            </div>
            <div className="edit-row" style={{ width: '100%' }}>
              <input
                className="input"
                type="text"
                placeholder="C:\Program Files (x86)\Steam"
                value={editPath}
                onChange={(e) => setEditPath(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && handleSave()}
                aria-label="Custom Steam root path"
              />
              <button className="btn btn-primary" onClick={handleSave} disabled={saving || !editPath.trim()}>Save</button>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Actions</span>
            </div>
            <div className="settings-row-right settings-row-actions">
              <button className="btn" onClick={handleAutoDetect} disabled={saving}>Detect Automatically</button>
              {steamRoot?.isCustom && (
                <button className="btn btn-danger" onClick={handleClear} disabled={saving}>Clear Custom Path</button>
              )}
            </div>
          </div>
        </div>

        <div className="info-box">
          <span className="info-box-title">Detection Order</span>
          <ol className="info-box-list">
            <li><code>STEAM_PATH</code> environment variable</li>
            <li>Windows Registry (<code>Valve\Steam</code>)</li>
            <li>Standard paths (<code>C:\Program Files (x86)\Steam</code>)</li>
          </ol>
        </div>
      </div>
    </div>
  );
}

function ExtensionsSettingsScreen({ plugins }: { plugins: PluginEntry[] }) {
  const enabledCount = plugins.filter((p) => p.enabled).length;
  return (
    <div className="view-content">
      <SettingsBreadcrumb category="Extensions" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">Extensions</h2>
        <p className="settings-category-desc">Manage installed extensions and their status.</p>
      </div>
      <div className="settings-sections">
        <div className="settings-group">
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Total Extensions</span>
              <span className="settings-row-desc">{plugins.length} extension(s) discovered</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Enabled</span>
              <span className="settings-row-desc">{enabledCount} extension(s) active</span>
            </div>
            <div className="settings-row-right">
              <span className="settings-status-badge settings-status-badge-auto">{enabledCount}</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Disabled</span>
              <span className="settings-row-desc">{plugins.length - enabledCount} extension(s) inactive</span>
            </div>
            <div className="settings-row-right">
              <span className="settings-status-badge">{plugins.length - enabledCount}</span>
            </div>
          </div>
        </div>

        {plugins.length > 0 && (
          <>
            <div className="settings-group-title">Installed Extensions</div>
            <div className="settings-group">
              {plugins.map((p) => (
                <div className="settings-row" key={p.id}>
                  <div className="settings-row-left">
                    <span className="settings-row-label">{p.name}</span>
                    <span className="settings-row-desc">
                      v{p.version}
                      {p.author && ` · ${p.author}`}
                      {p.source && ` · ${p.source}`}
                    </span>
                  </div>
                  <div className="settings-row-right">
                    <span className={`settings-status-badge ${p.enabled ? 'settings-status-badge-auto' : ''}`}>
                      {p.enabled ? 'enabled' : 'disabled'}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </>
        )}

        {plugins.length === 0 && (
          <div className="empty-state" style={{ padding: '32px 0' }}>
            <span className="empty-state-icon">
              <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
                <path d="M10 2v4M10 14v4M2 10h4M14 10h4" />
                <circle cx="10" cy="10" r="2.5" />
              </svg>
            </span>
            <span>No extensions installed</span>
            <span className="empty-state-hint">Place extensions in the plugins folder</span>
          </div>
        )}
      </div>
    </div>
  );
}

function AdvancedScreen() {
  const [log, setLog] = useState('');

  const appendLog = (msg: string) => {
    setLog((prev) => (prev ? prev + '\n' : '') + msg);
  };

  const handleClearCache = useCallback(async () => {
    try {
      await invoke('reload_plugins');
      appendLog('[OK] Lua engine cache cleared and plugins reloaded.');
    } catch (err) {
      appendLog(`[ERROR] Failed: ${err}`);
    }
  }, []);

  const handleRescan = useCallback(async () => {
    try {
      const result = await invoke('scan_plugins');
      appendLog(`[OK] Rescan complete — ${(result as unknown[]).length} plugins found.`);
    } catch (err) {
      appendLog(`[ERROR] Rescan failed: ${err}`);
    }
  }, []);

  const handleVerifySteam = useCallback(async () => {
    try {
      const info = await invoke<{ resolvedPath: string | null }>('get_steam_root');
      if (info.resolvedPath) {
        appendLog(`[OK] Steam root verified: ${info.resolvedPath}`);
      } else {
        appendLog('[WARN] Steam root not detected.');
      }
    } catch (err) {
      appendLog(`[ERROR] Steam verification failed: ${err}`);
    }
  }, []);

  return (
    <div className="view-content">
      <SettingsBreadcrumb category="Advanced" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">Advanced</h2>
        <p className="settings-category-desc">Cache management, rescan tools, and diagnostics.</p>
      </div>
      <div className="settings-sections">
        <div className="settings-group">
          <button className="settings-row settings-row-clickable" onClick={handleClearCache}>
            <div className="settings-row-left">
              <span className="settings-row-label">Clear Engine Cache</span>
              <span className="settings-row-desc">Reset all loaded Lua engines and reload extensions</span>
            </div>
            <div className="settings-row-right">
              <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                <polyline points="7 4 13 4 13 10" />
                <path d="M13 4L7 16" />
              </svg>
            </div>
          </button>
          <button className="settings-row settings-row-clickable" onClick={handleRescan}>
            <div className="settings-row-left">
              <span className="settings-row-label">Rescan Plugins</span>
              <span className="settings-row-desc">Force a fresh scan of the plugins directory</span>
            </div>
            <div className="settings-row-right">
              <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                <polyline points="7 4 13 4 13 10" />
                <path d="M13 4L7 16" />
              </svg>
            </div>
          </button>
          <button className="settings-row settings-row-clickable" onClick={handleVerifySteam}>
            <div className="settings-row-left">
              <span className="settings-row-label">Verify Steam</span>
              <span className="settings-row-desc">Check that the Steam root path is valid</span>
            </div>
            <div className="settings-row-right">
              <svg width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="6 10 9 13 14 7" />
                <circle cx="10" cy="10" r="7" />
              </svg>
            </div>
          </button>
        </div>

        {log && (
          <div className="fixes-log">
            <pre>{log}</pre>
          </div>
        )}
      </div>
    </div>
  );
}

function AboutScreen() {
  return (
    <div className="view-content">
      <SettingsBreadcrumb category="About" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">About</h2>
        <p className="settings-category-desc">Version, licenses, and application information.</p>
      </div>
      <div className="settings-sections">
        <div className="settings-group">
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Application</span>
              <span className="settings-row-desc">LumaForge Lite</span>
            </div>
            <div className="settings-row-right">
              <span className="settings-status-badge">v0.1.0</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Framework</span>
              <span className="settings-row-desc">Tauri v2 + React</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Lua Engine</span>
              <span className="settings-row-desc">Lua 5.4 via mlua</span>
            </div>
          </div>
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Purpose</span>
              <span className="settings-row-desc">System tray utility for managing Lua-based extensions for Steam CEF</span>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function SettingsBreadcrumb({ category }: { category: string }) {
  return (
    <div className="settings-breadcrumb-header">
      <nav className="settings-breadcrumb" aria-label="Breadcrumb">
        <span className="settings-breadcrumb-current">{category}</span>
      </nav>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Provider definitions (static metadata)
// ---------------------------------------------------------------------------
const PROVIDER_DEFS: ProviderDef[] = [
  {
    id: 'hubcapdb',
    name: 'HubcapDB',
    description: 'Primary database for Steam package manifests and Lua scripts.',
    capabilities: [],
    supportedTypes: [],
    requiresApiKey: true,
  },
  {
    id: 'ryuu',
    name: 'Ryuu',
    description: 'Alternative repack source. Adapter not yet available.',
    capabilities: [],
    supportedTypes: [],
    requiresApiKey: true,
  },
  {
    id: 'twentytwo',
    name: 'TwentyTwo Cloud',
    description: 'Cloud-hosted packages. Adapter not yet available.',
    capabilities: [],
    supportedTypes: [],
    requiresApiKey: false,
  },
  {
    id: 'sushi',
    name: 'Sushi',
    description: 'Community-maintained archive. Adapter not yet available.',
    capabilities: [],
    supportedTypes: [],
    requiresApiKey: false,
  },
  {
    id: 'custom',
    name: 'Custom API',
    description: 'Point to your own package server or API endpoint. Adapter not yet available.',
    capabilities: [],
    supportedTypes: [],
    requiresApiKey: false,
  },
];

function DownloadsPackagesView({ onBack }: { onBack: () => void }) {
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [multiFallback, setMultiFallback] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  // Load providers from backend on mount
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await invoke<{ ok: boolean; multiProviderFallback: boolean; providers: ProviderConfig[] }>('get_downloads_config');
        if (!cancelled) {
          setProviders(result.providers);
          setMultiFallback(result.multiProviderFallback);
        }
      } catch (err) {
        console.error('[luma-lite] Failed to load providers:', err);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const updateProvider = useCallback((id: string, patch: Partial<ProviderConfig>) => {
    setProviders((prev) =>
      prev.map((p) => (p.id === id ? { ...p, ...patch } : p))
    );
    setDirty(true);
  }, []);

  const handleSave = useCallback(async () => {
    setSaving(true);
    try {
      const result = await invoke<{ ok: boolean; providers: ProviderConfig[] }>('set_providers', { providers });
      if (result.ok) {
        setProviders(result.providers);
        await invoke('set_multi_provider_fallback', { enabled: multiFallback });
        setDirty(false);
      }
    } catch (err) {
      console.error('[luma-lite] Failed to save providers:', err);
    } finally {
      setSaving(false);
    }
  }, [providers, multiFallback]);

  const handleToggleFallback = useCallback(async (enabled: boolean) => {
    setMultiFallback(enabled);
    try {
      await invoke('set_multi_provider_fallback', { enabled });
    } catch (err) {
      console.error('[luma-lite] Failed to save fallback toggle:', err);
    }
  }, []);

  if (loading) {
    return (
      <div className="view-content">
        <SettingsBreadcrumb category="Downloads / Packages" />
        <div className="settings-category-header">
          <h2 className="settings-category-title">Downloads / Packages</h2>
          <p className="settings-category-desc">Manage download providers and package sources.</p>
        </div>
        <div className="loading-indicator"><div className="spinner" /></div>
      </div>
    );
  }

  return (
    <div className="view-content">
      <SettingsBreadcrumb category="Downloads / Packages" />
      <div className="settings-category-header">
        <h2 className="settings-category-title">Downloads / Packages</h2>
        <p className="settings-category-desc">Manage download providers and package sources.</p>
      </div>

      <div className="settings-sections">
        {/* Global toggle */}
        <div className="settings-group">
          <div className="settings-row">
            <div className="settings-row-left">
              <span className="settings-row-label">Multi-provider fallback</span>
              <span className="settings-row-desc">If one provider fails, automatically try the next enabled provider</span>
            </div>
            <div className="settings-row-right">
              <button
                className={`toggle ${multiFallback ? 'toggle-on' : ''}`}
                role="switch"
                aria-checked={multiFallback}
                onClick={() => handleToggleFallback(!multiFallback)}
              >
                <span className="toggle-track" />
              </button>
            </div>
          </div>
        </div>

        {/* Provider cards */}
        {providers.map((prov) => {
          const def = PROVIDER_DEFS.find((d) => d.id === prov.id);
          return (
            <ProviderCard
              key={prov.id}
              provider={prov}
              definition={def}
              onUpdate={updateProvider}
            />
          );
        })}

        {/* Save bar */}
        {dirty && (
          <div className="settings-save-bar">
            <button className="btn" onClick={() => { setDirty(false); invoke<{ ok: boolean; multiProviderFallback: boolean; providers: ProviderConfig[] }>('get_downloads_config').then((r) => { setProviders(r.providers); setMultiFallback(r.multiProviderFallback); }); }} disabled={saving}>
              Discard
            </button>
            <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
              {saving ? 'Saving\u2026' : 'Save Changes'}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProviderCard — reusable card for a single provider
// ---------------------------------------------------------------------------
function ProviderCard({
  provider,
  definition,
  onUpdate,
}: {
  provider: ProviderConfig;
  definition: ProviderDef | undefined;
  onUpdate: (id: string, patch: Partial<ProviderConfig>) => void;
}) {
  const [showKey, setShowKey] = useState(false);
  const caps = definition?.capabilities ?? [];
  const types = definition?.supportedTypes ?? [];
  const needsKey = definition?.requiresApiKey ?? false;

  return (
    <div className={`provider-card ${provider.enabled ? 'provider-card-enabled' : ''}`}>
      {/* Header row */}
      <div className="provider-card-header">
        <div className="provider-card-title-row">
          <button
            className={`toggle ${provider.enabled ? 'toggle-on' : ''}`}
            role="switch"
            aria-checked={provider.enabled}
            onClick={() => onUpdate(provider.id, { enabled: !provider.enabled })}
          >
            <span className="toggle-track" />
          </button>
          <span className="provider-card-name">{provider.name}</span>
          {provider.enabled && provider.hasApiKey && (
            <span className="provider-status-badge provider-status-online">Online</span>
          )}
          {provider.enabled && !provider.hasApiKey && needsKey && (
            <span className="provider-status-badge provider-status-warn">No API Key</span>
          )}
          {provider.enabled && provider.hasApiKey && !provider.adapterAvailable && (
            <span className="provider-status-badge provider-status-warn">Adapter unavailable</span>
          )}
          {provider.enabled && !needsKey && (
            <span className="provider-status-badge provider-status-online">Ready</span>
          )}
        </div>
        {definition?.description && (
          <span className="provider-card-desc">{definition.description}</span>
        )}
      </div>

      {/* Capability badges */}
      {caps.length > 0 && (
        <div className="provider-card-badges">
          {caps.map((cap) => (
            <span key={cap.label} className={`provider-cap-badge provider-cap-${cap.color}`}>
              {cap.label}
            </span>
          ))}
          {types.map((t) => (
            <span key={t} className="provider-type-badge">{t}</span>
          ))}
        </div>
      )}

      {/* Fields */}
      <div className="provider-card-fields">
        <div className="provider-field">
          <label className="provider-field-label">Base URL</label>
          <input
            className="input"
            type="text"
            placeholder="https://api.example.com/v1"
            value={provider.baseUrl}
            onChange={(e) => onUpdate(provider.id, { baseUrl: e.target.value })}
            disabled={!provider.enabled}
          />
        </div>
        {needsKey && (
          <div className="provider-field">
            <label className="provider-field-label">API Key</label>
            <div className="provider-key-row">
              <input
                className="input"
                type={showKey ? 'text' : 'password'}
                placeholder={provider.hasApiKey ? provider.keyPreview : 'Enter API key\u2026'}
                value={provider.apiKey ?? ''}
                onChange={(e) => onUpdate(provider.id, { apiKey: e.target.value })}
                disabled={!provider.enabled}
              />
              <button
                className="btn"
                onClick={() => setShowKey(!showKey)}
                disabled={!provider.enabled}
                aria-label={showKey ? 'Hide key' : 'Show key'}
              >
                {showKey ? (
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                    <path d="M1.5 8s2.5-4.5 6.5-4.5S14.5 8 14.5 8s-2.5 4.5-6.5 4.5S1.5 8 1.5 8z" />
                    <circle cx="8" cy="8" r="2" />
                  </svg>
                ) : (
                  <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
                    <path d="M2 2l12 12M6.5 6.8a2 2 0 002.7 2.7M1.5 8s2.5-4.5 6.5-4.5c.7 0 1.3.1 1.9.3M14.5 8s-1 1.8-2.8 3" />
                  </svg>
                )}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

export function SettingsView({ plugins, steamRoot, onSteamRootUpdated, appearance, onAppearanceUpdated }: SettingsViewProps) {
  const [subView, setSubView] = useState<SettingsSubView>(null);

  if (subView === 'appearance') {
    return <AppearanceView settings={appearance} onUpdate={onAppearanceUpdated} onBack={() => setSubView(null)} />;
  }

  if (subView === 'general') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">General</span>
          </nav>
        </div>
        <GeneralScreen plugins={plugins} steamRoot={steamRoot} />
      </div>
    );
  }

  if (subView === 'extensions') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">Extensions</span>
          </nav>
        </div>
        <ExtensionsSettingsScreen plugins={plugins} />
      </div>
    );
  }

  if (subView === 'integrations') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">Integrations</span>
          </nav>
        </div>
        <IntegrationsScreen steamRoot={steamRoot} onSteamRootUpdated={onSteamRootUpdated} />
      </div>
    );
  }

  if (subView === 'downloads') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">Downloads / Packages</span>
          </nav>
        </div>
        <DownloadsPackagesView onBack={() => setSubView(null)} />
      </div>
    );
  }

  if (subView === 'advanced') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">Advanced</span>
          </nav>
        </div>
        <AdvancedScreen />
      </div>
    );
  }

  if (subView === 'about') {
    return (
      <div className="view-content">
        <div className="settings-breadcrumb-header">
          <nav className="settings-breadcrumb" aria-label="Breadcrumb">
            <button className="settings-back-btn" onClick={() => setSubView(null)} aria-label="Back to Settings">
              <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="10 3 5 8 10 13" />
              </svg>
            </button>
            <button className="settings-breadcrumb-link" onClick={() => setSubView(null)}>Settings</button>
            <span className="settings-breadcrumb-sep"><ChevronRight /></span>
            <span className="settings-breadcrumb-current">About</span>
          </nav>
        </div>
        <AboutScreen />
      </div>
    );
  }

  return <SettingsHome onSelect={setSubView} plugins={plugins} steamRoot={steamRoot} />;
}
