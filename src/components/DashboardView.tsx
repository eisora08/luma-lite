import {
  useCallback,
  useMemo,
  useState,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ConfirmModal } from './ConfirmModal';
import type {
  PluginEntry,
  SteamRootInfo,
} from '../types';
import type {
  SteamRuntimeStatus,
} from '../App';

interface SteamRuntimeOperationResult {
  ok: boolean;
  status: string;
  message: string;
  steamRunning: boolean;
  cefDebuggingEnabled: boolean;
  cefDebugPort: number | null;
}

interface DashboardViewProps {
  plugins: PluginEntry[];
  steamRoot: SteamRootInfo | null;
  bridgeRunning: boolean;
  steamRuntime: SteamRuntimeStatus | null;
  steamRuntimeLoading: boolean;
  onRefreshSteamRuntime: () => Promise<void>;
}

type SteamOperation =
  | 'start'
  | 'restart'
  | null;

function RefreshIcon() {
  return (
    <svg
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M3 10a7 7 0 0113.2-3.2" />
      <path d="M17 10a7 7 0 01-13.2 3.2" />
      <polyline points="3 4 3 8 7 8" />
      <polyline points="17 16 17 12 13 12" />
    </svg>
  );
}

function PlayIcon() {
  return (
    <svg
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M6 4l10 6-10 6V4z" />
    </svg>
  );
}

function FolderIcon() {
  return (
    <svg
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M2 5a2 2 0 012-2h3l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2H4a2 2 0 01-2-2V5z" />
    </svg>
  );
}

function StatusCard({
  label,
  value,
  description,
  tag,
  healthy,
  title,
}: {
  label: string;
  value: string;
  description?: string;
  tag?: string;
  healthy: boolean;
  title?: string;
}) {
  return (
    <div
      className="status-card"
      title={title}
    >
      <div className="status-card-header">
        <span
          className={`status-indicator ${
            healthy
              ? 'status-ok'
              : 'status-warn'
          }`}
        />

        <span className="status-label">
          {label}
        </span>
      </div>

      <span className="status-value">
        {value}
      </span>

      {description && (
        <span className="status-path">
          {description}
        </span>
      )}

      {tag && (
        <span className="status-tag">
          {tag}
        </span>
      )}
    </div>
  );
}

export function DashboardView({
  plugins,
  steamRoot,
  bridgeRunning,
  steamRuntime,
  steamRuntimeLoading,
  onRefreshSteamRuntime,
}: DashboardViewProps) {
  const [
    steamOperation,
    setSteamOperation,
  ] = useState<SteamOperation>(null);

  const [
    restartModalOpen,
    setRestartModalOpen,
  ] = useState(false);

  const [
    runtimeMessage,
    setRuntimeMessage,
  ] = useState<string | null>(null);

  const [
    runtimeError,
    setRuntimeError,
  ] = useState<string | null>(null);

  const [
    reloadingExtensions,
    setReloadingExtensions,
  ] = useState(false);

  const enabledCount = useMemo(
    () =>
      plugins.filter(
        (plugin) => plugin.enabled
      ).length,
    [plugins]
  );

  const cefConnected =
    steamRuntime?.cefDebuggingEnabled === true;

  const runtimeBusy =
    steamOperation !== null;

  const cefStatus = useMemo(() => {
    if (
      steamRuntimeLoading &&
      !steamRuntime
    ) {
      return {
        value: 'Checking Steam...',
        description:
          'Detecting the Steam process and CEF debugger.',
        healthy: false,
        tag: 'checking',
      };
    }

    if (!steamRuntime) {
      return {
        value: 'Status unavailable',
        description:
          'LumaForge could not read the Steam runtime status.',
        healthy: false,
        tag: 'unknown',
      };
    }

    if (
      steamRuntime.cefDebuggingEnabled
    ) {
      return {
        value:
          steamRuntime.cefDebugPort !== null
            ? `Connected :${steamRuntime.cefDebugPort}`
            : 'Connected',
        description:
          'Steam Store integration is ready.',
        healthy: true,
        tag: 'ready',
      };
    }

    if (steamRuntime.restartRequired) {
      return {
        value: 'Restart required',
        description:
          'Steam is running without CEF debugging.',
        healthy: false,
        tag: 'action required',
      };
    }

    if (steamRuntime.steamRunning) {
      return {
        value: 'CEF unavailable',
        description:
          'Steam is running, but the debugger is unavailable.',
        healthy: false,
        tag: 'offline',
      };
    }

    if (
      !steamRuntime.steamExecutableFound
    ) {
      return {
        value: 'Steam not found',
        description:
          'Configure the Steam installation path in Settings.',
        healthy: false,
        tag: 'not found',
      };
    }

    return {
      value: 'Steam is closed',
      description:
        'Start Steam with CEF debugging to enable Store integration.',
      healthy: false,
      tag: 'stopped',
    };
  }, [
    steamRuntime,
    steamRuntimeLoading,
  ]);

  const refreshRuntimeState =
    useCallback(async () => {
      try {
        await onRefreshSteamRuntime();
      } catch (error) {
        setRuntimeError(
          error instanceof Error
            ? error.message
            : String(error)
        );
      }
    }, [onRefreshSteamRuntime]);

  const startSteamWithCef =
    useCallback(async () => {
      if (runtimeBusy) {
        return;
      }

      setSteamOperation('start');
      setRuntimeMessage(null);
      setRuntimeError(null);

      try {
        const result =
          await invoke<SteamRuntimeOperationResult>(
            'start_steam_with_cef'
          );

        if (!result.ok) {
          setRuntimeError(result.message);
          return;
        }

        setRuntimeMessage(result.message);
        await refreshRuntimeState();
      } catch (error) {
        setRuntimeError(
          error instanceof Error
            ? error.message
            : String(error)
        );
      } finally {
        setSteamOperation(null);
      }
    }, [
      runtimeBusy,
      refreshRuntimeState,
    ]);

  const restartSteamWithCef =
    useCallback(async () => {
      if (runtimeBusy) {
        return;
      }

      setSteamOperation('restart');
      setRuntimeMessage(null);
      setRuntimeError(null);

      try {
        const result =
          await invoke<SteamRuntimeOperationResult>(
            'restart_steam_with_cef'
          );

        if (!result.ok) {
          setRuntimeError(result.message);
          return;
        }

        setRuntimeMessage(result.message);
        setRestartModalOpen(false);

        await refreshRuntimeState();
      } catch (error) {
        setRuntimeError(
          error instanceof Error
            ? error.message
            : String(error)
        );
      } finally {
        setSteamOperation(null);
      }
    }, [
      runtimeBusy,
      refreshRuntimeState,
    ]);

  const reloadExtensions =
    useCallback(async () => {
      if (reloadingExtensions) {
        return;
      }

      setReloadingExtensions(true);

      try {
        await invoke('reload_plugins');
        await refreshRuntimeState();
      } catch (error) {
        console.error(
          '[luma-lite] Failed to reload extensions:',
          error
        );
      } finally {
        setReloadingExtensions(false);
      }
    }, [
      reloadingExtensions,
      refreshRuntimeState,
    ]);

  const openPluginsFolder =
    useCallback(async () => {
      try {
        await invoke(
          'extension_open_plugins_folder'
        );
      } catch (error) {
        console.error(
          '[luma-lite] Failed to open plugins folder:',
          error
        );
      }
    }, []);

  const openRestartModal =
    useCallback(() => {
      if (runtimeBusy) {
        return;
      }

      setRuntimeMessage(null);
      setRuntimeError(null);
      setRestartModalOpen(true);
    }, [runtimeBusy]);

  const closeRestartModal =
    useCallback(() => {
      if (runtimeBusy) {
        return;
      }

      setRestartModalOpen(false);
    }, [runtimeBusy]);

  const renderSteamAction = () => {
    if (
      steamRuntimeLoading &&
      !steamRuntime
    ) {
      return (
        <button
          type="button"
          className="action-card"
          disabled
          aria-label="Checking Steam status"
        >
          <span className="action-card-icon">
            <RefreshIcon />
          </span>

          <div className="action-card-text">
            <span className="action-card-label">
              Checking Steam
            </span>

            <span className="action-card-desc">
              Detecting CEF debugging status
            </span>
          </div>
        </button>
      );
    }

    if (!steamRuntime) {
      return (
        <button
          type="button"
          className="action-card"
          onClick={() => {
            void refreshRuntimeState();
          }}
          disabled={runtimeBusy}
        >
          <span className="action-card-icon">
            <RefreshIcon />
          </span>

          <div className="action-card-text">
            <span className="action-card-label">
              Refresh Steam Status
            </span>

            <span className="action-card-desc">
              Steam runtime status is unavailable
            </span>
          </div>
        </button>
      );
    }

    if (cefConnected) {
      return (
        <button
          type="button"
          className="action-card"
          onClick={() => {
            void refreshRuntimeState();
          }}
          disabled={runtimeBusy}
          aria-label="Refresh Steam CEF status"
        >
          <span className="action-card-icon">
            <RefreshIcon />
          </span>

          <div className="action-card-text">
            <span className="action-card-label">
              Refresh Steam Status
            </span>

            <span className="action-card-desc">
              CEF connected
              {steamRuntime.cefDebugPort !== null
                ? ` on port ${steamRuntime.cefDebugPort}`
                : ''}
            </span>
          </div>
        </button>
      );
    }

    if (steamRuntime.restartRequired) {
      return (
        <button
          type="button"
          className="action-card"
          onClick={openRestartModal}
          disabled={runtimeBusy}
          aria-label="Restart Steam with CEF debugging"
        >
          <span className="action-card-icon">
            <RefreshIcon />
          </span>

          <div className="action-card-text">
            <span className="action-card-label">
              {steamOperation === 'restart'
                ? 'Restarting Steam...'
                : 'Restart Steam with CEF'}
            </span>

            <span className="action-card-desc">
              {steamOperation === 'restart'
                ? 'Waiting for Steam and the CEF debugger'
                : 'Required to enable Steam Store integration'}
            </span>
          </div>
        </button>
      );
    }

    if (
      !steamRuntime.steamRunning &&
      steamRuntime.steamExecutableFound
    ) {
      return (
        <button
          type="button"
          className="action-card"
          onClick={() => {
            void startSteamWithCef();
          }}
          disabled={runtimeBusy}
          aria-label="Start Steam with CEF debugging"
        >
          <span className="action-card-icon">
            <PlayIcon />
          </span>

          <div className="action-card-text">
            <span className="action-card-label">
              {steamOperation === 'start'
                ? 'Starting Steam...'
                : 'Start Steam with CEF'}
            </span>

            <span className="action-card-desc">
              {steamOperation === 'start'
                ? 'Waiting for the CEF debugger'
                : 'Launch Steam with Store integration enabled'}
            </span>
          </div>
        </button>
      );
    }

    return (
      <button
        type="button"
        className="action-card"
        onClick={() => {
          void refreshRuntimeState();
        }}
        disabled={runtimeBusy}
        aria-label="Refresh Steam runtime status"
      >
        <span className="action-card-icon">
          <RefreshIcon />
        </span>

        <div className="action-card-text">
          <span className="action-card-label">
            Refresh Steam Status
          </span>

          <span className="action-card-desc">
            Check the Steam process and CEF debugger again
          </span>
        </div>
      </button>
    );
  };

  return (
    <>
      <div className="view-content">
        <div className="page-header">
          <div className="page-header-left">
            <h2 className="view-title">
              System Status
            </h2>

            <p className="view-desc">
              Overview of LumaForge services,
              Steam integration, and extensions.
            </p>
          </div>
        </div>

        <div className="status-grid">
          <StatusCard
            label="Steam Bridge"
            value={
              bridgeRunning
                ? 'Active :21775'
                : 'Offline'
            }
            description={
              bridgeRunning
                ? 'Local communication bridge is ready.'
                : 'The local bridge is unavailable.'
            }
            tag={
              bridgeRunning
                ? 'ready'
                : 'unavailable'
            }
            healthy={bridgeRunning}
          />

          <StatusCard
            label="Steam CEF"
            value={cefStatus.value}
            description={
              cefStatus.description
            }
            tag={cefStatus.tag}
            healthy={cefStatus.healthy}
            title={cefStatus.description}
          />

          <StatusCard
            label="Steam Root"
            value={
              steamRoot?.resolvedPath
                ? steamRoot.resolvedPath
                    .split(/[/\\]/)
                    .slice(-2)
                    .join('/')
                : 'Not detected'
            }
            description={
              steamRoot?.isCustom
                ? 'Using a custom Steam installation path.'
                : steamRoot?.resolvedPath
                  ? 'Steam installation detected.'
                  : 'Configure Steam in Settings.'
            }
            tag={
              steamRoot?.isCustom
                ? 'custom'
                : steamRoot?.resolvedPath
                  ? 'detected'
                  : 'not found'
            }
            healthy={
              Boolean(
                steamRoot?.resolvedPath
              )
            }
            title={
              steamRoot?.resolvedPath ??
              'Steam root not found'
            }
          />

          <StatusCard
            label="Extensions"
            value={`${enabledCount}/${plugins.length} enabled`}
            description={
              plugins.length > 0
                ? 'Extension configuration loaded.'
                : 'No extensions were detected.'
            }
            tag={
              plugins.length > 0
                ? 'loaded'
                : 'none'
            }
            healthy={plugins.length > 0}
          />
        </div>

        {(runtimeMessage || runtimeError) && (
          <div
            className={`runtime-notice ${
              runtimeError
                ? 'runtime-notice--error'
                : 'runtime-notice--success'
            }`}
            role={
              runtimeError
                ? 'alert'
                : 'status'
            }
            aria-live="polite"
          >
            <span className="runtime-notice-text">
              {runtimeError ??
                runtimeMessage}
            </span>

            <button
              type="button"
              className="runtime-notice-close"
              aria-label="Dismiss message"
              onClick={() => {
                setRuntimeError(null);
                setRuntimeMessage(null);
              }}
            >
              ×
            </button>
          </div>
        )}

        <div className="section-header">
          Steam Integration
        </div>

        <div className="quick-actions">
          {renderSteamAction()}
        </div>

        <div className="section-header">
          Quick Actions
        </div>

        <div className="quick-actions">
          <button
            type="button"
            className="action-card"
            onClick={() => {
              void reloadExtensions();
            }}
            disabled={reloadingExtensions}
            aria-label="Reload all extensions"
          >
            <span className="action-card-icon">
              <RefreshIcon />
            </span>

            <div className="action-card-text">
              <span className="action-card-label">
                {reloadingExtensions
                  ? 'Reloading Extensions...'
                  : 'Reload All Extensions'}
              </span>

              <span className="action-card-desc">
                Refresh loaded extension state
              </span>
            </div>
          </button>

          <button
            type="button"
            className="action-card"
            onClick={() => {
              void openPluginsFolder();
            }}
            aria-label="Open plugins folder"
          >
            <span className="action-card-icon">
              <FolderIcon />
            </span>

            <div className="action-card-text">
              <span className="action-card-label">
                Open Plugins Folder
              </span>

              <span className="action-card-desc">
                Browse installed plugin files
              </span>
            </div>
          </button>
        </div>
      </div>

      <ConfirmModal
        open={restartModalOpen}
        title="Restart Steam with CEF?"
        description={
          <>
            Steam is currently running without
            CEF debugging. LumaForge must restart
            Steam to activate the
            <strong> Store integration</strong>.
          </>
        }
        warning={
          'Make sure no game, installation, or download is currently active before continuing.'
        }
        confirmLabel="RESTART STEAM"
        cancelLabel="NOT NOW"
        busyLabel="RESTARTING..."
        tone="warning"
        busy={
          steamOperation === 'restart'
        }
        autoFocus="cancel"
        closeOnBackdrop
        closeOnEscape
        onCancel={closeRestartModal}
        onConfirm={
          restartSteamWithCef
        }
      />
    </>
  );
}