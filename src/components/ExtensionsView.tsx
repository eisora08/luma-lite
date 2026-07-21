import { useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { PluginEntry } from '../types';

interface ExtensionsViewProps {
  plugins: PluginEntry[];
  loading: boolean;
  onToggle: (plugin: PluginEntry) => Promise<boolean>;
  onReload: () => void;
}

export function ExtensionsView({ plugins, loading, onToggle, onReload }: ExtensionsViewProps) {
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [errorIds, setErrorIds] = useState<Set<string>>(new Set());
  const pendingRef = useRef(pendingIds);

  const handleToggle = useCallback(async (plugin: PluginEntry) => {
    if (pendingRef.current.has(plugin.id)) return;

    setPendingIds((prev) => {
      const next = new Set(prev);
      next.add(plugin.id);
      pendingRef.current = next;
      return next;
    });
    setErrorIds((prev) => {
      const next = new Set(prev);
      next.delete(plugin.id);
      return next;
    });

    try {
      const success = await onToggle(plugin);
      if (!success) {
        setErrorIds((prev) => {
          const next = new Set(prev);
          next.add(plugin.id);
          return next;
        });
      }
    } catch {
      setErrorIds((prev) => {
        const next = new Set(prev);
        next.add(plugin.id);
        return next;
      });
    } finally {
      setPendingIds((prev) => {
        const next = new Set(prev);
        next.delete(plugin.id);
        pendingRef.current = next;
        return next;
      });
    }
  }, [onToggle]);

  const handleOpenFolder = useCallback(async () => {
    try {
      await invoke('extension_open_plugins_folder');
    } catch (err) {
      console.error('[luma-lite] Failed to open plugins folder:', err);
    }
  }, []);

  const enabledCount = plugins.filter((p) => p.enabled).length;

  return (
    <div className="view-content">
      <div className="page-header">
        <div className="page-header-left">
          <h2 className="view-title">Extensions</h2>
          <p className="view-desc">
            {plugins.length === 0
              ? 'No extensions installed yet.'
              : `${enabledCount} of ${plugins.length} extensions enabled.`}
          </p>
        </div>
        <div className="view-actions">
          <button className="btn btn-primary" onClick={onReload} disabled={loading}>
            <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
              <path d="M3 10a7 7 0 0113.2-3.2" />
              <path d="M17 10a7 7 0 01-13.2 3.2" />
              <polyline points="3 4 3 8 7 8" />
              <polyline points="17 16 17 12 13 12" />
            </svg>
            Reload
          </button>
          <button className="btn" onClick={handleOpenFolder}>
            <svg width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
              <path d="M2 5a2 2 0 012-2h3l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2H4a2 2 0 01-2-2V5z" />
            </svg>
            Folder
          </button>
        </div>
      </div>

      <div className="extensions-list">
        {loading ? (
          <div className="loading-indicator">
            <div className="spinner" />
            Scanning plugins...
          </div>
        ) : plugins.length === 0 ? (
          <div className="empty-state">
            <span className="empty-state-icon">
              <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
                <path d="M10 2v4M10 14v4M2 10h4M14 10h4" />
                <circle cx="10" cy="10" r="2.5" />
              </svg>
            </span>
            <span>No extensions found</span>
            <span className="empty-state-hint">Place extensions in the plugins folder</span>
          </div>
        ) : (
          plugins.map((plugin, i) => {
            const isPending = pendingIds.has(plugin.id);
            const hasError = errorIds.has(plugin.id);
            return (
              <div
                className="extension-card"
                key={plugin.id}
                style={{ animationDelay: `${Math.min(i * 50, 250)}ms` }}
              >
                <div className="extension-card-row">
                  <span className={`ext-dot ${plugin.enabled ? 'ext-dot-on' : 'ext-dot-off'}`} />
                  <div className="extension-info">
                    <span className="extension-name">{plugin.name}</span>
                    <span className="extension-meta">
                      v{plugin.version}
                      {plugin.author && ` · ${plugin.author}`}
                      {plugin.source && ` · ${plugin.source}`}
                    </span>
                  </div>
                  <button
                    className={`toggle ${isPending ? 'toggle-loading' : ''} ${hasError ? 'toggle-error' : ''}`}
                    role="switch"
                    aria-checked={plugin.enabled}
                    aria-label={`${plugin.enabled ? 'Disable' : 'Enable'} ${plugin.name}`}
                    disabled={isPending}
                    onClick={() => handleToggle(plugin)}
                  >
                    <span className="toggle-track" />
                  </button>
                </div>
                {plugin.description && (
                  <p className="extension-desc">{plugin.description}</p>
                )}
                {hasError && (
                  <p className="extension-error">
                    Failed to {plugin.enabled ? 'disable' : 'enable'} this extension.
                  </p>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
