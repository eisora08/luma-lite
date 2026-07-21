import {
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Sidebar } from './components/Sidebar';
import { DashboardView } from './components/DashboardView';
import { ExtensionsView } from './components/ExtensionsView';
import { SettingsView } from './components/SettingsView';
import type {
  AppearanceSettings,
  PluginEntry,
  SteamRootInfo,
  ViewId,
} from './types';
import {
  DEFAULT_APPEARANCE as DEFAULT_APP,
} from './types';
import './App.css';

const VIEW_TITLES: Record<ViewId, string> = {
  dashboard: 'Dashboard',
  extensions: 'Extensions',
  settings: 'Settings',
};

export type SteamRuntimeStatus = {
  steamRunning: boolean;
  cefDebuggingEnabled: boolean;
  cefDebugPort: number | null;
  restartRequired: boolean;
  steamExecutableFound: boolean;
  steamExecutable: string | null;
};

function App() {
  const [activeView, setActiveView] =
    useState<ViewId>('dashboard');

  const [plugins, setPlugins] =
    useState<PluginEntry[]>([]);

  const [steamRoot, setSteamRoot] =
    useState<SteamRootInfo | null>(null);

  const [loading, setLoading] =
    useState(true);

  const [
    sidebarExpanded,
    setSidebarExpanded,
  ] = useState(false);

  const [bridgeRunning, setBridgeRunning] =
    useState(false);

  const [appearance, setAppearance] =
    useState<AppearanceSettings>(DEFAULT_APP);

  const [steamRuntime, setSteamRuntime] =
    useState<SteamRuntimeStatus | null>(null);

  const [
    steamRuntimeLoading,
    setSteamRuntimeLoading,
  ] = useState(true);

  /*
   * Prevent overlapping runtime and bridge requests.
   * This is especially important while Steam is starting
   * or restarting and CEF is not ready yet.
   */
  const steamRuntimeRequestPending =
    useRef(false);

  const bridgeRequestPending =
    useRef(false);

  const win = getCurrentWindow();

  const fetchPlugins = useCallback(async () => {
    try {
      setLoading(true);

      const result = await invoke<{
        plugins: PluginEntry[];
      }>('list_plugins');

      setPlugins(result.plugins);
    } catch (error) {
      console.error(
        '[luma-lite] Failed to load plugins:',
        error
      );
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchSteamRoot =
    useCallback(async () => {
      try {
        const info = await invoke<SteamRootInfo>(
          'get_steam_root'
        );

        setSteamRoot(info);
      } catch (error) {
        console.error(
          '[luma-lite] Failed to get Steam root:',
          error
        );
      }
    }, []);

  const fetchBridge = useCallback(
    async (): Promise<void> => {
      if (bridgeRequestPending.current) {
        return;
      }

      bridgeRequestPending.current = true;

      try {
        const status = await invoke<{
          running: boolean;
          port: number;
        }>('get_bridge_status');

        setBridgeRunning(status.running);
      } catch (error) {
        console.error(
          '[luma-lite] Failed to get bridge status:',
          error
        );

        setBridgeRunning(false);
      } finally {
        bridgeRequestPending.current = false;
      }
    },
    []
  );

  /*
   * showLoading should be true only during the initial
   * application load. Background checks remain silent.
   */
  const fetchSteamRuntime = useCallback(
    async (
      showLoading = false
    ): Promise<void> => {
      if (steamRuntimeRequestPending.current) {
        return;
      }

      steamRuntimeRequestPending.current = true;

      if (showLoading) {
        setSteamRuntimeLoading(true);
      }

      try {
        const status =
          await invoke<SteamRuntimeStatus>(
            'get_steam_runtime_status'
          );

        setSteamRuntime(status);
      } catch (error) {
        console.error(
          '[luma-lite] Failed to get Steam runtime status:',
          error
        );

        setSteamRuntime(null);
      } finally {
        steamRuntimeRequestPending.current =
          false;

        if (showLoading) {
          setSteamRuntimeLoading(false);
        }
      }
    },
    []
  );

  const fetchAppearance =
    useCallback(async () => {
      try {
        const settings =
          await invoke<AppearanceSettings>(
            'get_appearance_settings'
          );

        setAppearance(settings);
      } catch (error) {
        console.error(
          '[luma-lite] Failed to load appearance:',
          error
        );
      }
    }, []);

  /*
   * Initial application loading.
   * All independent requests run in parallel.
   */
  useEffect(() => {
    void Promise.all([
      fetchPlugins(),
      fetchSteamRoot(),
      fetchBridge(),
      fetchAppearance(),
      fetchSteamRuntime(true),
    ]);
  }, [
    fetchPlugins,
    fetchSteamRoot,
    fetchBridge,
    fetchAppearance,
    fetchSteamRuntime,
  ]);

  /*
   * Poll only while Steam integration is incomplete.
   *
   * Polling stops when:
   * - the local bridge is active, and
   * - Steam CEF debugging is connected.
   */
  useEffect(() => {
    const integrationReady =
      steamRuntime?.cefDebuggingEnabled === true &&
      bridgeRunning;

    if (integrationReady) {
      return;
    }

    const intervalId = window.setInterval(() => {
      if (
        document.visibilityState !== 'visible'
      ) {
        return;
      }

      void fetchSteamRuntime(false);
      void fetchBridge();
    }, 5000);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [
    steamRuntime?.cefDebuggingEnabled,
    bridgeRunning,
    fetchSteamRuntime,
    fetchBridge,
  ]);

  /*
   * Refresh when the LumaForge window becomes visible
   * again, but only if integration is not already ready.
   */
  useEffect(() => {
    function handleVisibilityChange() {
      if (
        document.visibilityState !== 'visible'
      ) {
        return;
      }

      const integrationReady =
        steamRuntime?.cefDebuggingEnabled ===
          true &&
        bridgeRunning;

      if (integrationReady) {
        return;
      }

      void fetchSteamRuntime(false);
      void fetchBridge();
    }

    document.addEventListener(
      'visibilitychange',
      handleVisibilityChange
    );

    return () => {
      document.removeEventListener(
        'visibilitychange',
        handleVisibilityChange
      );
    };
  }, [
    steamRuntime?.cefDebuggingEnabled,
    bridgeRunning,
    fetchSteamRuntime,
    fetchBridge,
  ]);

  useEffect(() => {
    document.documentElement.setAttribute(
      'data-theme',
      appearance.theme
    );

    document.documentElement.setAttribute(
      'data-surface',
      appearance.surfaceStyle
    );

    document.documentElement.setAttribute(
      'data-density',
      appearance.density
    );

    document.documentElement.setAttribute(
      'data-reduce-motion',
      String(appearance.reduceMotion)
    );
  }, [appearance]);

  const handleAppearanceUpdate = useCallback(
    async (settings: AppearanceSettings) => {
      setAppearance(settings);

      try {
        await invoke(
          'set_appearance_settings',
          {
            settings,
          }
        );
      } catch (error) {
        console.error(
          '[luma-lite] Failed to save appearance:',
          error
        );
      }
    },
    []
  );

  const handleToggle = useCallback(
    async (
      plugin: PluginEntry
    ): Promise<boolean> => {
      const previousEnabled = plugin.enabled;
      const desiredEnabled = !plugin.enabled;

      console.log(
        `[luma-lite] handleToggle called for ${plugin.id}: ${previousEnabled} -> ${desiredEnabled}`
      );

      setPlugins((previousPlugins) =>
        previousPlugins.map(
          (currentPlugin) =>
            currentPlugin.id === plugin.id
              ? {
                  ...currentPlugin,
                  enabled: desiredEnabled,
                }
              : currentPlugin
        )
      );

      try {
        const updated =
          await invoke<PluginEntry>(
            'toggle_plugin',
            {
              extensionId: plugin.id,
              enabled: desiredEnabled,
            }
          );

        setPlugins((previousPlugins) =>
          previousPlugins.map(
            (currentPlugin) =>
              currentPlugin.id === updated.id
                ? updated
                : currentPlugin
          )
        );

        /*
         * Refresh runtime state after enabling or
         * disabling a plugin.
         */
        await Promise.all([
          fetchBridge(),
          fetchSteamRuntime(false),
        ]);

        return true;
      } catch (error) {
        console.error(
          '[luma-lite] Toggle failed:',
          error
        );

        setPlugins((previousPlugins) =>
          previousPlugins.map(
            (currentPlugin) =>
              currentPlugin.id === plugin.id
                ? {
                    ...currentPlugin,
                    enabled: previousEnabled,
                  }
                : currentPlugin
          )
        );

        return false;
      }
    },
    [
      fetchBridge,
      fetchSteamRuntime,
    ]
  );

  const handleReload =
    useCallback(async () => {
      try {
        await invoke('reload_plugins');
        await fetchPlugins();

        await Promise.all([
          fetchBridge(),
          fetchSteamRuntime(false),
        ]);
      } catch (error) {
        console.error(
          '[luma-lite] Reload failed:',
          error
        );
      }
    }, [
      fetchPlugins,
      fetchBridge,
      fetchSteamRuntime,
    ]);

  const refreshSteamIntegration =
    useCallback(async (): Promise<void> => {
      await Promise.all([
        fetchSteamRuntime(false),
        fetchBridge(),
      ]);
    }, [
      fetchSteamRuntime,
      fetchBridge,
    ]);

  const integrationOnline =
    bridgeRunning &&
    steamRuntime?.cefDebuggingEnabled === true;

  const integrationStatusText =
    steamRuntimeLoading
      ? 'Checking'
      : integrationOnline
        ? steamRuntime?.cefDebugPort !== null &&
          steamRuntime?.cefDebugPort !== undefined
          ? `Online · ${steamRuntime.cefDebugPort}`
          : 'Online'
        : steamRuntime?.restartRequired
          ? 'Restart required'
          : steamRuntime?.steamRunning
            ? 'CEF offline'
            : 'Steam offline';

  const integrationStatusTitle =
    steamRuntimeLoading
      ? 'Checking Steam integration'
      : integrationOnline
        ? `Steam CEF connected on port ${
            steamRuntime?.cefDebugPort ??
            'unknown'
          }`
        : steamRuntime?.restartRequired
          ? 'Steam is running without CEF debugging'
          : steamRuntime?.steamRunning
            ? 'Steam is running, but CEF debugging is unavailable'
            : steamRuntime?.steamExecutableFound
              ? 'Steam is not running'
              : 'Steam executable could not be found';

  return (
    <div className="app-shell">
      <Sidebar
        activeView={activeView}
        onViewChange={setActiveView}
        pluginCount={plugins.length}
        expanded={sidebarExpanded}
        onToggleExpand={() => {
          setSidebarExpanded(
            (currentValue) => !currentValue
          );
        }}
      />

      <main className="app-main">
        <header
          className="app-header"
          data-tauri-drag-region
        >
          <div className="header-left">
            <span className="app-header-title">
              LumaForge
            </span>

            <div className="header-breadcrumb">
              <span className="header-breadcrumb-sep">
                /
              </span>

              <span className="header-breadcrumb-current">
                {VIEW_TITLES[activeView]}
              </span>
            </div>
          </div>

          <div className="header-spacer" />

          <div className="header-right">
            <div
              className="header-status"
              aria-live="polite"
              title={integrationStatusTitle}
            >
              <span
                className={`header-status-dot ${
                  integrationOnline
                    ? ''
                    : 'header-status-dot-offline'
                }`}
              />

              <span className="header-status-text">
                {integrationStatusText}
              </span>
            </div>

            <span className="app-header-version">
              v0.1.0
            </span>

            <button
              type="button"
              className="window-btn window-btn-minimize"
              data-tauri-drag-region="noDrag"
              onClick={() => {
                void win.minimize();
              }}
              aria-label="Minimize window"
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                aria-hidden="true"
              >
                <line
                  x1="2"
                  y1="6"
                  x2="10"
                  y2="6"
                />
              </svg>
            </button>

            <button
              type="button"
              className="window-btn window-btn-close"
              data-tauri-drag-region="noDrag"
              onClick={() => {
                void win.hide();
              }}
              aria-label="Close window"
            >
              <svg
                width="12"
                height="12"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.2"
                strokeLinecap="round"
                aria-hidden="true"
              >
                <line
                  x1="3"
                  y1="3"
                  x2="9"
                  y2="9"
                />

                <line
                  x1="9"
                  y1="3"
                  x2="3"
                  y2="9"
                />
              </svg>
            </button>
          </div>
        </header>

        <div className="app-view">
          {activeView === 'dashboard' && (
            <DashboardView
              plugins={plugins}
              steamRoot={steamRoot}
              bridgeRunning={bridgeRunning}
              steamRuntime={steamRuntime}
              steamRuntimeLoading={
                steamRuntimeLoading
              }
              onRefreshSteamRuntime={
                refreshSteamIntegration
              }
            />
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
              onAppearanceUpdated={
                handleAppearanceUpdate
              }
            />
          )}
        </div>
      </main>
    </div>
  );
}

export default App;