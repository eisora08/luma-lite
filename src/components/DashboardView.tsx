import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { SteamRootInfo, BridgeStatus, PluginEntry } from '../types';

interface DashboardViewProps {
  plugins: PluginEntry[];
  steamRoot: SteamRootInfo | null;
}

export function DashboardView({ plugins, steamRoot }: DashboardViewProps) {
  const [bridge, setBridge] = useState<BridgeStatus | null>(null);

  useEffect(() => {
    invoke<BridgeStatus>('get_bridge_status')
      .then(setBridge)
      .catch(() => {});
  }, []);

  const enabledCount = plugins.filter((p) => p.enabled).length;

  return (
    <div className="view-content">
      <div className="page-header">
        <div className="page-header-left">
          <h2 className="view-title">System Status</h2>
          <p className="view-desc">Overview of LumaForge Lite services and extensions.</p>
        </div>
      </div>

      <div className="status-grid">
        <div className="status-card">
          <div className="status-card-header">
            <span className={`status-indicator ${bridge?.running ? 'status-ok' : 'status-warn'}`} />
            <span className="status-label">Steam Bridge</span>
          </div>
          <span className="status-value">
            {bridge?.running ? `Active :${bridge.port}` : 'Offline'}
          </span>
        </div>

        <div className="status-card">
          <div className="status-card-header">
            <span className={`status-indicator ${steamRoot?.resolvedPath ? 'status-ok' : 'status-warn'}`} />
            <span className="status-label">Steam Root</span>
          </div>
          <span className="status-value status-path" title={steamRoot?.resolvedPath ?? 'Not found'}>
            {steamRoot?.resolvedPath
              ? steamRoot.resolvedPath.split(/[/\\]/).slice(-2).join('/')
              : 'Not detected'}
          </span>
          {steamRoot?.isCustom && <span className="status-tag">custom</span>}
        </div>

        <div className="status-card">
          <div className="status-card-header">
            <span className={`status-indicator ${plugins.length > 0 ? 'status-ok' : 'status-warn'}`} />
            <span className="status-label">Extensions</span>
          </div>
          <span className="status-value">
            {enabledCount}/{plugins.length} enabled
          </span>
        </div>
      </div>

      <div className="section-header">Quick Actions</div>
      <div className="quick-actions">
        <button
          className="action-card"
          onClick={() => invoke('reload_plugins')}
          aria-label="Reload all extensions"
        >
          <span className="action-card-icon">
            <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
              <path d="M3 10a7 7 0 0113.2-3.2" />
              <path d="M17 10a7 7 0 01-13.2 3.2" />
              <polyline points="3 4 3 8 7 8" />
              <polyline points="17 16 17 12 13 12" />
            </svg>
          </span>
          <div className="action-card-text">
            <span className="action-card-label">Reload All Extensions</span>
            <span className="action-card-desc">Refresh all loaded Lua engines</span>
          </div>
        </button>
        <button
          className="action-card"
          onClick={() => invoke('extension_open_plugins_folder')}
          aria-label="Open plugins folder"
        >
          <span className="action-card-icon">
            <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
              <path d="M2 5a2 2 0 012-2h3l2 2h7a2 2 0 012 2v8a2 2 0 01-2 2H4a2 2 0 01-2-2V5z" />
            </svg>
          </span>
          <div className="action-card-text">
            <span className="action-card-label">Open Plugins Folder</span>
            <span className="action-card-desc">Browse installed plugin files</span>
          </div>
        </button>
      </div>
    </div>
  );
}
