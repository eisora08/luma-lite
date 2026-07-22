import { useState, useCallback, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ConfirmModal } from './ConfirmModal';
import type { PluginEntry, RepositorySource, RepositoryExtensionView } from '../types';

const BUILTIN_IDS = new Set(['steam-store-helper', 'opensteamtool']);

interface ExtensionsViewProps {
  plugins: PluginEntry[];
  loading: boolean;
  onToggle: (plugin: PluginEntry) => Promise<boolean>;
  onReload: () => void;
}

type TabId = 'installed' | 'browse';

export function ExtensionsView({ plugins, loading, onToggle, onReload }: ExtensionsViewProps) {
  const [pendingIds, setPendingIds] = useState<Set<string>>(new Set());
  const [errorIds, setErrorIds] = useState<Set<string>>(new Set());
  const pendingRef = useRef(pendingIds);

  const [activeTab, setActiveTab] = useState<TabId>('installed');

  const [repositories, setRepositories] = useState<RepositorySource[]>([]);
  const [repoExtensions, setRepoExtensions] = useState<RepositoryExtensionView[]>([]);
  const [repoLoading, setRepoLoading] = useState(false);
  const [repoErrors, setRepoErrors] = useState<Map<string, string>>(new Map());

  const [addRepoUrl, setAddRepoUrl] = useState('');
  const [addRepoLabel, setAddRepoLabel] = useState('');
  const [addRepoBusy, setAddRepoBusy] = useState(false);
  const [addRepoError, setAddRepoError] = useState<string | null>(null);

  const [refreshingUrl, setRefreshingUrl] = useState<string | null>(null);

  const [confirmModal, setConfirmModal] = useState<{
    open: boolean;
    title: string;
    description: string;
    confirmLabel: string;
    busyLabel: string;
    tone: 'default' | 'warning' | 'danger';
    onConfirm: () => Promise<void>;
  } | null>(null);

  const [installingId, setInstallingId] = useState<string | null>(null);
  const [uninstallingId, setUninstallingId] = useState<string | null>(null);

  // Stale-response guard: bump generation on each call so only the latest response applies
  const generationRef = useRef(0);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Single canonical load function that fetches repos + extensions together
  const loadRepositoriesAndExtensions = useCallback(async () => {
    const gen = ++generationRef.current;
    setRepoLoading(true);
    setRepoErrors(new Map());

    try {
      // list_repository_extensions internally ensures indexes are loaded
      const result = await invoke<{
        extensions: RepositoryExtensionView[];
        repositories: RepositorySource[];
      }>('list_repository_extensions');

      // Discard if a newer call started
      if (!mountedRef.current || gen !== generationRef.current) return;

      setRepoExtensions(result.extensions);
      setRepositories(result.repositories);
    } catch (err) {
      if (!mountedRef.current || gen !== generationRef.current) return;
      console.error('[luma-lite] Failed to load repository extensions:', err);
    } finally {
      if (mountedRef.current && gen === generationRef.current) {
        setRepoLoading(false);
      }
    }
  }, []);

  // On mount: load installed plugins + repository extensions
  useEffect(() => {
    void loadRepositoriesAndExtensions();
  }, [loadRepositoriesAndExtensions]);

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

  const handleAddRepository = useCallback(async () => {
    if (!addRepoUrl.trim()) return;

    setAddRepoBusy(true);
    setAddRepoError(null);

    try {
      // add_extension_repository now fetches+validates+persists the index atomically
      const source = await invoke<RepositorySource>('add_extension_repository', {
        url: addRepoUrl.trim(),
        label: addRepoLabel.trim() || undefined,
      });

      setAddRepoUrl('');
      setAddRepoLabel('');

      // Immediately reload — the backend already cached the index
      await loadRepositoriesAndExtensions();

      // Switch to Browse tab so the user sees the new extensions
      setActiveTab('browse');
    } catch (err) {
      setAddRepoError(err instanceof Error ? err.message : String(err));
    } finally {
      setAddRepoBusy(false);
    }
  }, [addRepoUrl, addRepoLabel, loadRepositoriesAndExtensions]);

  const handleRefreshRepository = useCallback(async (url: string) => {
    setRefreshingUrl(url);
    try {
      await invoke('refresh_extension_repository', { url });
      await loadRepositoriesAndExtensions();
    } catch (err) {
      console.error('[luma-lite] Failed to refresh repository:', err);
    } finally {
      setRefreshingUrl(null);
    }
  }, [loadRepositoriesAndExtensions]);

  const handleRemoveRepository = useCallback((url: string) => {
    setConfirmModal({
      open: true,
      title: 'Remove Repository',
      description: 'Remove this repository from your configured sources? Installed extensions will not be affected.',
      confirmLabel: 'Remove',
      busyLabel: 'Removing...',
      tone: 'warning',
      onConfirm: async () => {
        try {
          await invoke('remove_extension_repository', { url });
          await loadRepositoriesAndExtensions();
        } finally {
          setConfirmModal(null);
        }
      },
    });
  }, [loadRepositoriesAndExtensions]);

  const handleInstallExtension = useCallback((ext: RepositoryExtensionView) => {
    setConfirmModal({
      open: true,
      title: `Install ${ext.name}`,
      description: `Install extension "${ext.name}" v${ext.version} from ${ext.repositoryLabel || ext.repositoryUrl}?`,
      confirmLabel: 'Install',
      busyLabel: 'Installing...',
      tone: 'default',
      onConfirm: async () => {
        setInstallingId(ext.id);
        try {
          await invoke('install_repository_extension', {
            extensionId: ext.id,
            manifestUrl: ext.manifestUrl,
          });
          await onReload();
          await loadRepositoriesAndExtensions();
        } finally {
          setInstallingId(null);
          setConfirmModal(null);
        }
      },
    });
  }, [onReload, loadRepositoriesAndExtensions]);

  const handleUninstallExtension = useCallback((plugin: PluginEntry) => {
    setConfirmModal({
      open: true,
      title: `Uninstall ${plugin.name}`,
      description: `Uninstall extension "${plugin.name}"? This will remove all files.`,
      confirmLabel: 'Uninstall',
      busyLabel: 'Uninstalling...',
      tone: 'danger',
      onConfirm: async () => {
        setUninstallingId(plugin.id);
        try {
          await invoke('uninstall_extension', { extensionId: plugin.id });
          await onReload();
          await loadRepositoriesAndExtensions();
        } finally {
          setUninstallingId(null);
          setConfirmModal(null);
        }
      },
    });
  }, [onReload, loadRepositoriesAndExtensions]);

  const enabledCount = plugins.filter((p) => p.enabled).length;
  const availableExtensions = repoExtensions.filter((ext) => !ext.installed);
  const installedRepoExtensions = repoExtensions.filter((ext) => ext.installed);

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

      <div className="extensions-tabs">
        <button
          className={`extensions-tab ${activeTab === 'installed' ? 'extensions-tab-active' : ''}`}
          onClick={() => setActiveTab('installed')}
        >
          Installed
          {plugins.length > 0 && (
            <span className="extensions-tab-count">{plugins.length}</span>
          )}
        </button>
        <button
          className={`extensions-tab ${activeTab === 'browse' ? 'extensions-tab-active' : ''}`}
          onClick={() => setActiveTab('browse')}
        >
          Browse
          {availableExtensions.length > 0 && (
            <span className="extensions-tab-count">{availableExtensions.length}</span>
          )}
        </button>
      </div>

      {activeTab === 'installed' && (
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
              <span className="empty-state-hint">Place extensions in the plugins folder or browse the repository</span>
            </div>
          ) : (
            plugins.map((plugin, i) => {
              const isPending = pendingIds.has(plugin.id);
              const hasError = errorIds.has(plugin.id);
              const isBuiltin = BUILTIN_IDS.has(plugin.id);
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
                        {plugin.source === 'local' && ' · local'}
                        {plugin.source === 'repository' && ' · repository'}
                        {isBuiltin && ' · built-in'}
                      </span>
                    </div>
                    <div className="extension-actions">
                      {!isBuiltin && plugin.source === 'repository' && (
                        <button
                          className="btn btn-danger btn-sm"
                          disabled={uninstallingId === plugin.id || isPending}
                          onClick={() => handleUninstallExtension(plugin)}
                        >
                          {uninstallingId === plugin.id ? 'Removing...' : 'Uninstall'}
                        </button>
                      )}
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
      )}

      {activeTab === 'browse' && (
        <>
          <div className="extensions-repos-section">
            <div className="extensions-repos-header">
              <h3 className="extensions-section-title">Repositories</h3>
              <button className="btn btn-sm" onClick={() => void loadRepositoriesAndExtensions()} disabled={repoLoading}>
                {repoLoading ? 'Refreshing...' : 'Refresh All'}
              </button>
            </div>

            {repositories.length > 0 ? (
              <div className="extensions-repos-list">
                {repositories.map((repo) => (
                  <div className="extensions-repo-item" key={repo.url}>
                    <div className="extensions-repo-info">
                      <span className="extensions-repo-name">
                        {repo.label || repo.url}
                      </span>
                      <span className="extensions-repo-meta">
                        {repo.lastFetched
                          ? `Updated ${new Date(repo.lastFetched * 1000).toLocaleDateString()}`
                          : 'Never fetched'}
                        {repo.lastError && (
                          <span className="extension-error"> · {repo.lastError}</span>
                        )}
                      </span>
                    </div>
                    <div className="extensions-repo-actions">
                      <button
                        className="btn btn-sm"
                        disabled={refreshingUrl === repo.url}
                        onClick={() => void handleRefreshRepository(repo.url)}
                      >
                        {refreshingUrl === repo.url ? 'Refreshing...' : 'Refresh'}
                      </button>
                      <button
                        className="btn btn-danger btn-sm"
                        onClick={() => handleRemoveRepository(repo.url)}
                      >
                        Remove
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              !repoLoading && (
                <p className="extensions-section-empty">No repositories configured</p>
              )
            )}

            <div className="extensions-add-repo">
              <input
                type="text"
                className="extensions-add-repo-input"
                placeholder="Repository URL (https://...)"
                value={addRepoUrl}
                onChange={(e) => setAddRepoUrl(e.target.value)}
                disabled={addRepoBusy}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && addRepoUrl.trim() && !addRepoBusy) {
                    void handleAddRepository();
                  }
                }}
              />
              <input
                type="text"
                className="extensions-add-repo-input extensions-add-repo-label"
                placeholder="Label (optional)"
                value={addRepoLabel}
                onChange={(e) => setAddRepoLabel(e.target.value)}
                disabled={addRepoBusy}
              />
              <button
                className="btn btn-primary btn-sm"
                disabled={!addRepoUrl.trim() || addRepoBusy}
                onClick={() => void handleAddRepository()}
              >
                {addRepoBusy ? 'Adding...' : 'Add'}
              </button>
            </div>
            {addRepoError && (
              <p className="extension-error">{addRepoError}</p>
            )}
          </div>

          <div className="extensions-browse-section">
            <h3 className="extensions-section-title">Available Extensions</h3>

            {repoLoading ? (
              <div className="loading-indicator">
                <div className="spinner" />
                Loading extensions...
              </div>
            ) : availableExtensions.length === 0 ? (
              <div className="empty-state">
                <span className="empty-state-icon">
                  <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
                    <circle cx="10" cy="10" r="7" />
                    <path d="M10 6v4M10 14v.5" />
                  </svg>
                </span>
                <span>No extensions available</span>
                <span className="empty-state-hint">
                  {repositories.length === 0
                    ? 'Add a repository to browse available extensions'
                    : 'All available extensions are already installed'}
                </span>
              </div>
            ) : (
              availableExtensions.map((ext, i) => (
                <div
                  className="extension-card"
                  key={ext.id}
                  style={{ animationDelay: `${Math.min(i * 50, 250)}ms` }}
                >
                  <div className="extension-card-row">
                    <div className="extension-info">
                      <span className="extension-name">
                        {ext.name}
                        {ext.verified && (
                          <span className="extension-verified" title="Verified extension"> ✓</span>
                        )}
                      </span>
                      <span className="extension-meta">
                        v{ext.version}
                        {ext.author && ` · ${ext.author}`}
                        {ext.repositoryLabel && ` · ${ext.repositoryLabel}`}
                      </span>
                    </div>
                    <button
                      className="btn btn-primary btn-sm"
                      disabled={installingId === ext.id}
                      onClick={() => void handleInstallExtension(ext)}
                    >
                      {installingId === ext.id ? 'Installing...' : 'Install'}
                    </button>
                  </div>
                  {ext.description && (
                    <p className="extension-desc">{ext.description}</p>
                  )}
                </div>
              ))
            )}
          </div>
        </>
      )}

      {confirmModal && (
        <ConfirmModal
          open={confirmModal.open}
          title={confirmModal.title}
          description={confirmModal.description}
          confirmLabel={confirmModal.confirmLabel}
          busyLabel={confirmModal.busyLabel}
          tone={confirmModal.tone}
          onConfirm={confirmModal.onConfirm}
          onCancel={() => setConfirmModal(null)}
        />
      )}
    </div>
  );
}
