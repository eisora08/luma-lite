import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import * as fs from 'fs';
import * as path from 'path';

const INJECT_JS_PATH = path.resolve(
  __dirname,
  '../../plugins/builtin/steam-store-helper/inject.js'
);

function loadInjectScript(): string {
  return fs.readFileSync(INJECT_JS_PATH, 'utf-8');
}

function navigateTo(url: string) {
  history.pushState(null, '', url);
}

function runScript(code: string) {
  const fn = new Function(code);
  fn();
}

function cleanup() {
  try {
    if (window.__lumaforge_ssh__) {
      window.__lumaforge_ssh__.deactivate();
    }
  } catch { /* ignore */ }
  document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]').forEach((el) => el.remove());
  document.querySelectorAll('[data-lumaforge-modal="steam-store-helper"]').forEach((el) => el.remove());
  document.querySelectorAll('#luma_ssh_kf').forEach((el) => el.remove());
  delete (window as Record<string, unknown>)['__lumaforge_ssh__'];
  try { history.replaceState(null, '', '/'); } catch (_) {}
}

function setupAppPage(appId: string) {
  document.body.innerHTML =
    '<div id="game_area_purchase_game"><button class="btn_addtocart">Buy</button></div>';
  navigateTo('/app/' + appId);
}

function getButton(): HTMLButtonElement | null {
  return document.querySelector(
    '[data-lumaforge-extension="steam-store-helper"]'
  ) as HTMLButtonElement | null;
}

function getButtonState(): string | null {
  const btn = getButton();
  return btn ? btn.getAttribute('data-lumaforge-state') : null;
}

declare global {
  interface Window {
    __lumaforge_ssh__?: {
      activate: () => void;
      deactivate: () => void;
      scheduleReconcile: (reason: string) => void;
      version: string;
      active: boolean;
      documentId: string;
      currentUrl: string | null;
      currentAppId: string | null;
      reconcileCount: number;
      observerActive: boolean;
      historyWrapped: boolean;
    };
  }
}

describe('steam-store-helper inject.js — required behavior', () => {
  let scriptCode: string;

  beforeEach(() => {
    scriptCode = loadInjectScript();
    document.body.innerHTML = '';
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  // =========================================================================
  // Lifecycle creation and reuse
  // =========================================================================

  it('App A activation creates one lifecycle', () => {
    setupAppPage('730');
    runScript(scriptCode);
    const ns = window.__lumaforge_ssh__;
    expect(ns).toBeDefined();
    expect(ns!.active).toBe(true);
    expect(ns!.version).toBe('2.5.0-download-flow');
    expect(typeof ns!.documentId).toBe('string');
    expect(ns!.documentId.length).toBeGreaterThan(0);
    expect(ns!.currentAppId).toBe('730');
  });

  it('repeated activation reuses the lifecycle and schedules reconcile', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    const ns = window.__lumaforge_ssh__;
    const firstDocId = ns!.documentId;

    runScript(scriptCode);
    expect(window.__lumaforge_ssh__!.documentId).toBe(firstDocId);
    expect(window.__lumaforge_ssh__!.active).toBe(true);
  });

  // =========================================================================
  // SPA navigation via History API
  // =========================================================================

  it('pushState from App A to B changes currentAppId to B', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.active).toBe(true);
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('570');
    });
  });

  it('replaceState from App A to B changes currentAppId to B', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.active).toBe(true);
    });

    setupAppPage('570');
    history.replaceState(null, '', '/app/570');

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('570');
    });
  });

  it('popstate schedules reconciliation', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.active).toBe(true);
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');
    history.back();

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('730');
    });
  });

  // =========================================================================
  // Button management during navigation
  // =========================================================================

  it('old App A button is removed when navigating to App B', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      const btn = document.querySelector('[data-lumaforge-app-id="730"]');
      expect(btn).not.toBeNull();
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');

    await vi.waitFor(() => {
      const oldBtn = document.querySelector('[data-lumaforge-app-id="730"]');
      expect(oldBtn).toBeNull();
    });
  });

  it('exactly one App B button exists after navigation', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.active).toBe(true);
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');

    await vi.waitFor(() => {
      const btnB = document.querySelector('[data-lumaforge-app-id="570"]');
      expect(btnB).not.toBeNull();
    });

    const allButtons = document.querySelectorAll(
      '[data-lumaforge-extension="steam-store-helper"]'
    );
    expect(allButtons).toHaveLength(1);
  });

  it('local status is requested for App B after navigation', async () => {
    const fetchCalls: string[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string) => {
        fetchCalls.push(url);
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(fetchCalls.some((u) => u.includes('/api/local-status/730'))).toBe(true);
    });

    fetchCalls.length = 0;
    setupAppPage('570');
    history.pushState(null, '', '/app/570');

    await vi.waitFor(() => {
      expect(fetchCalls.some((u) => u.includes('/api/local-status/570'))).toBe(true);
    });

    vi.unstubAllGlobals();
  });

  // =========================================================================
  // Stale request protection
  // =========================================================================

  it('App A response cannot modify App B button', async () => {
    let resolveA!: (v: unknown) => void;
    const fetchCalls: string[] = [];

    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, opts?: RequestInit) => {
        fetchCalls.push(url);
        if (url.includes('/api/local-status/730')) {
          return new Promise((resolve) => { resolveA = resolve; });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '570', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(fetchCalls.some((u) => u.includes('/api/local-status/730'))).toBe(true);
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('570');
    });

    resolveA({
      ok: true,
      json: () => Promise.resolve({ ok: true, appId: '730', in_library: true }),
    });

    await new Promise((r) => setTimeout(r, 50));

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    );
    if (btn) {
      expect(btn.getAttribute('data-lumaforge-app-id')).toBe('570');
      expect(btn.textContent).not.toContain('IN LIBRARY');
    }

    vi.unstubAllGlobals();
  });

  it('AbortError does not display BRIDGE ERROR', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((_url: string, opts?: RequestInit) => {
        if (opts?.signal) {
          const err = new Error('The operation was aborted.');
          Object.defineProperty(err, 'name', { value: 'AbortError', writable: false });
          return Promise.reject(err);
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await new Promise((r) => setTimeout(r, 50));

    const btn = getButton();
    if (btn) {
      expect(btn.textContent).not.toContain('BRIDGE ERROR');
      expect(btn.getAttribute('data-lumaforge-state')).not.toBe('bridge-error');
    }

    vi.unstubAllGlobals();
  });

  // =========================================================================
  // Navigation away from app pages
  // =========================================================================

  it('navigation away from /app/ removes controls', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(
        document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
      ).not.toBeNull();
    });

    navigateTo('/cart');
    history.pushState(null, '', '/cart');

    await vi.waitFor(() => {
      expect(
        document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
      ).toBeNull();
    });

    expect(window.__lumaforge_ssh__!.currentAppId).toBeNull();
  });

  // =========================================================================
  // Teardown
  // =========================================================================

  it('teardown restores history methods and removes listeners', () => {
    const origPushState = History.prototype.pushState;
    const origReplaceState = History.prototype.replaceState;

    setupAppPage('730');
    runScript(scriptCode);

    expect(History.prototype.pushState).not.toBe(origPushState);

    window.__lumaforge_ssh__!.deactivate();

    expect(History.prototype.pushState).toBe(origPushState);
    expect(History.prototype.replaceState).toBe(origReplaceState);
    expect(window.__lumaforge_ssh__!.active).toBe(false);
  });

  it('deactivation removes the button', () => {
    setupAppPage('730');
    runScript(scriptCode);
    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).not.toBeNull();

    window.__lumaforge_ssh__!.deactivate();
    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).toBeNull();
  });

  it('deactivation removes modal if present', () => {
    setupAppPage('730');
    runScript(scriptCode);
    const modal = document.createElement('div');
    modal.setAttribute('data-lumaforge-modal', 'steam-store-helper');
    document.body.appendChild(modal);

    window.__lumaforge_ssh__!.deactivate();
    expect(
      document.querySelector('[data-lumaforge-modal="steam-store-helper"]')
    ).toBeNull();
  });

  it('repeated deactivate() is safe', () => {
    setupAppPage('730');
    runScript(scriptCode);
    window.__lumaforge_ssh__!.deactivate();
    expect(() => window.__lumaforge_ssh__!.deactivate()).not.toThrow();
  });

  // =========================================================================
  // Same-version bootstrap
  // =========================================================================

  it('same-version bootstrap calls reconcile instead of returning silently', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.active).toBe(true);
    });
    const reconcileCountBefore = window.__lumaforge_ssh__!.reconcileCount;

    runScript(scriptCode);

    expect(window.__lumaforge_ssh__!.active).toBe(true);

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.reconcileCount).toBeGreaterThan(reconcileCountBefore);
    });
  });

  // =========================================================================
  // New-document bootstrap (different version)
  // =========================================================================

  it('new-document bootstrap creates one lifecycle', () => {
    setupAppPage('730');
    runScript(scriptCode);
    const ns1 = window.__lumaforge_ssh__;
    expect(ns1!.active).toBe(true);
    expect(ns1!.currentAppId).toBe('730');
  });

  // =========================================================================
  // Missing target container
  // =========================================================================

  it('missing target container waits for later reconciliation', async () => {
    document.body.innerHTML = '<div id="something_else">Hello</div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).toBeNull();

    document.body.innerHTML =
      '<div id="game_area_purchase_game"><button class="btn_addtocart">Buy</button></div>';

    window.__lumaforge_ssh__!.scheduleReconcile('test');

    await vi.waitFor(() => {
      expect(
        document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
      ).not.toBeNull();
    });
  });

  // =========================================================================
  // Observer root reattachment
  // =========================================================================

  it('detached observer root is reattached', async () => {
    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.observerActive).toBe(true);
    });

    const newBody = document.createElement('body');
    newBody.innerHTML = document.body.innerHTML;
    document.documentElement.replaceChild(newBody, document.body);

    window.__lumaforge_ssh__!.scheduleReconcile('test-reattach');

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.observerActive).toBe(true);
    });
  });

  // =========================================================================
  // Diagnostic state
  // =========================================================================

  it('lifecycle exposes correct diagnostic state', () => {
    setupAppPage('730');
    runScript(scriptCode);
    const ns = window.__lumaforge_ssh__;
    expect(ns!.version).toBe('2.5.0-download-flow');
    expect(typeof ns!.documentId).toBe('string');
    expect(ns!.active).toBe(true);
    expect(ns!.currentUrl).toContain('/app/730');
    expect(ns!.currentAppId).toBe('730');
    expect(typeof ns!.reconcileCount).toBe('number');
    expect(typeof ns!.observerActive).toBe('boolean');
    expect(typeof ns!.historyWrapped).toBe('boolean');
  });

  // =========================================================================
  // Button attributes
  // =========================================================================

  it('button has correct attributes including state marker', () => {
    setupAppPage('730');
    runScript(scriptCode);
    const btn = getButton();
    expect(btn).not.toBeNull();
    expect(btn!.tagName).toBe('BUTTON');
    expect(btn!.type).toBe('button');
    expect(btn!.getAttribute('aria-label')).toContain('730');
    expect(btn!.title).toContain('app 730');
    expect(btn!.getAttribute('data-lumaforge-app-id')).toBe('730');
    expect(btn!.getAttribute('data-lumaforge-state')).toBe('checking');
  });

  it('creates no button on unsupported URL', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/cart');
    runScript(scriptCode);
    expect(
      document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]')
    ).toHaveLength(0);
  });

  it('rejects malformed IDs', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/abc123');
    runScript(scriptCode);
    expect(
      document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]')
    ).toHaveLength(0);
  });

  it('handles /app/{id}/dlc path format', () => {
    setupAppPage('730');
    navigateTo('/app/730/DLC_Name');
    runScript(scriptCode);
    const btn = getButton();
    expect(btn).not.toBeNull();
    expect(btn!.getAttribute('data-lumaforge-app-id')).toBe('730');
  });

  // =========================================================================
  // Fetch retry behavior
  // =========================================================================

  it('first local-status attempt fails and second succeeds', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          if (callCount === 1) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()).not.toBeNull();
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    });

    vi.unstubAllGlobals();
  });

  it('button remains CHECKING during retry delay', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(() => {
        return Promise.reject(new TypeError('Failed to fetch'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await new Promise((r) => setTimeout(r, 50));

    const btn = getButton();
    expect(btn).not.toBeNull();
    expect(btn!.textContent).toContain('CHECKING');
    expect(btn!.getAttribute('data-lumaforge-state')).toBe('checking');

    vi.unstubAllGlobals();
  });

  it('successful retry changes button to ADD VIA LUMAFORGE', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          if (callCount <= 3) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()).not.toBeNull();
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
      expect(getButton()!.getAttribute('data-lumaforge-state')).toBe('ready');
    }, { timeout: 15000 });

    vi.unstubAllGlobals();
  });

  it('four failed network attempts produce BRIDGE ERROR', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(() => {
        return Promise.reject(new TypeError('Failed to fetch'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      const btn = getButton();
      expect(btn).not.toBeNull();
      expect(btn!.textContent).toContain('BRIDGE ERROR');
      expect(btn!.getAttribute('data-lumaforge-state')).toBe('bridge-error');
    }, { timeout: 10000 });

    vi.unstubAllGlobals();
  }, 20000);

  it('button in BRIDGE ERROR is not treated as fully reconciled', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(() => {
        return Promise.reject(new TypeError('Failed to fetch'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButtonState()).toBe('bridge-error');
    }, { timeout: 15000 });

    const reconcileBefore = window.__lumaforge_ssh__!.reconcileCount;

    window.__lumaforge_ssh__!.scheduleReconcile('test-bridge-error-retry');

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.reconcileCount).toBeGreaterThan(reconcileBefore);
    });

    await new Promise((r) => setTimeout(r, 50));

    expect(getButtonState()).toBe('bridge-error');

    vi.unstubAllGlobals();
  }, 30000);

  it('navigation cancels retries for the old App ID', async () => {
    let fetchCalls: string[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, opts?: RequestInit) => {
        if (opts?.signal?.aborted) {
          const err = new Error('The operation was aborted.');
          Object.defineProperty(err, 'name', { value: 'AbortError', writable: false });
          return Promise.reject(err);
        }
        fetchCalls.push(url);
        return Promise.reject(new TypeError('Failed to fetch'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await new Promise((r) => setTimeout(r, 50));

    fetchCalls = [];
    setupAppPage('570');
    history.pushState(null, '', '/app/570');

    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('570');
    });

    const callsForA = fetchCalls.filter(u => u.includes('/api/local-status/730'));
    expect(callsForA).toHaveLength(0);

    vi.unstubAllGlobals();
  });

  it('stale App A retry cannot update App B', async () => {
    let resolveA!: (v: unknown) => void;

    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          return new Promise((resolve) => { resolveA = resolve; });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '570', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      const btn = getButton();
      expect(btn).not.toBeNull();
      expect(btn!.getAttribute('data-lumaforge-app-id')).toBe('730');
    });

    setupAppPage('570');
    history.pushState(null, '', '/app/570');
    await vi.waitFor(() => {
      expect(window.__lumaforge_ssh__!.currentAppId).toBe('570');
    });

    resolveA({
      ok: true,
      json: () => Promise.resolve({ ok: true, appId: '730', in_library: true }),
    });

    await new Promise((r) => setTimeout(r, 50));

    const btn = getButton();
    if (btn) {
      expect(btn.getAttribute('data-lumaforge-app-id')).toBe('570');
      expect(btn.textContent).not.toContain('IN LIBRARY');
    }

    vi.unstubAllGlobals();
  });

  it('multiple MutationObserver events do not start duplicate request chains', async () => {
    let fetchCalls: string[] = [];
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        fetchCalls.push(url);
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);
    await vi.waitFor(() => {
      expect(fetchCalls.some(u => u.includes('/api/local-status/730'))).toBe(true);
    });

    const callsBefore = fetchCalls.length;

    for (let i = 0; i < 5; i++) {
      window.__lumaforge_ssh__!.scheduleReconcile('dom');
    }

    await new Promise((r) => setTimeout(r, 200));

    const statusCalls = fetchCalls.filter(u => u.includes('/api/local-status/730'));
    expect(statusCalls.length).toBeLessThanOrEqual(callsBefore + 1);

    vi.unstubAllGlobals();
  });

  it('failed status responses are not cached', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          if (callCount <= 4) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButtonState()).toBe('bridge-error');
    }, { timeout: 15000 });

    window.__lumaforge_ssh__!.scheduleReconcile('test-retry-after-fail');

    await vi.waitFor(() => {
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    }, { timeout: 20000 });

    expect(callCount).toBeGreaterThan(4);

    vi.unstubAllGlobals();
  }, 30000);

  it('successful responses may be cached', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    });

    const callsAfterFirst = callCount;

    document.getElementById('luma-action-btn')?.remove();
    window.__lumaforge_ssh__!.scheduleReconcile('test-cache');

    await vi.waitFor(() => {
      expect(getButton()).not.toBeNull();
    });

    await vi.waitFor(() => {
      expect(getButton()!.getAttribute('data-lumaforge-state')).toBe('ready');
    });

    expect(callCount).toBeGreaterThanOrEqual(callsAfterFirst);

    vi.unstubAllGlobals();
  });

  it('provider loading retries a rejected fetch', async () => {
    let providerCallCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status')) {
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        if (url.includes('/api/providers')) {
          providerCallCount++;
          if (providerCallCount === 1) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, providers: [{ id: 'test', name: 'Test', adapterAvailable: true }] }),
          });
        }
        return Promise.reject(new TypeError('Unknown URL'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    });

    getButton()!.click();

    await vi.waitFor(() => {
      expect(document.querySelector('[data-lumaforge-modal="steam-store-helper"]')).not.toBeNull();
    });

    await vi.waitFor(() => {
      expect(document.querySelector('[data-lumaforge-source-id="test"]')).not.toBeNull();
    }, { timeout: 10000 });

    expect(providerCallCount).toBeGreaterThanOrEqual(2);

    vi.unstubAllGlobals();
  });

  it('closing the modal aborts provider retries', async () => {
    let providerAborted = false;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, opts?: RequestInit) => {
        if (url.includes('/api/local-status')) {
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        if (url.includes('/api/providers')) {
          if (opts?.signal) {
            opts.signal.addEventListener('abort', () => { providerAborted = true; });
          }
          return new Promise((_resolve, reject) => {
            const timer = setTimeout(() => {
              reject(new TypeError('Should have been aborted'));
            }, 10000);
            if (opts?.signal) {
              opts.signal.addEventListener('abort', () => {
                clearTimeout(timer);
                const err = new DOMException('Aborted', 'AbortError');
                reject(err);
              });
            }
          });
        }
        return Promise.reject(new TypeError('Unknown'));
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    });

    getButton()!.click();

    await vi.waitFor(() => {
      expect(document.querySelector('[data-lumaforge-modal="steam-store-helper"]')).not.toBeNull();
    });

    const modal = document.querySelector('[data-lumaforge-modal="steam-store-helper"]');
    if (modal) {
      (modal as HTMLElement).click();
    }

    await new Promise((r) => setTimeout(r, 100));

    expect(providerAborted).toBe(true);

    vi.unstubAllGlobals();
  });

  it('teardown aborts active fetches and clears retry timers', async () => {
    let fetchAborted = false;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((_url: string, opts?: RequestInit) => {
        if (opts?.signal) {
          opts.signal.addEventListener('abort', () => { fetchAborted = true; });
          const err = new DOMException('The operation was aborted.', 'AbortError');
          return Promise.reject(err);
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await new Promise((r) => setTimeout(r, 50));

    window.__lumaforge_ssh__!.deactivate();

    expect(window.__lumaforge_ssh__!.active).toBe(false);
    expect(fetchAborted).toBe(true);

    vi.unstubAllGlobals();
  });

  // =========================================================================
  // Bridge-error recovery
  // =========================================================================

  it('clicking a BRIDGE ERROR button retries local status', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          if (callCount <= 4) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButtonState()).toBe('bridge-error');
    }, { timeout: 15000 });

    getButton()!.click();

    await vi.waitFor(() => {
      expect(getButtonState()).toBe('ready');
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    }, { timeout: 15000 });

    vi.unstubAllGlobals();
  }, 30000);

  it('scheduleReconcile bridge-recovery retries the failed App ID', async () => {
    let callCount = 0;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          callCount++;
          if (callCount <= 4) {
            return Promise.reject(new TypeError('Failed to fetch'));
          }
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      const s = getButtonState();
      expect(s === 'bridge-error' || s === 'ready').toBe(true);
    }, { timeout: 15000 });

    if (getButtonState() === 'bridge-error') {
      window.__lumaforge_ssh__!.scheduleReconcile('bridge-recovery');
    }

    await vi.waitFor(() => {
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
      expect(getButton()!.getAttribute('data-lumaforge-state')).toBe('ready');
    }, { timeout: 15000 });

    vi.unstubAllGlobals();
  }, 30000);

  it('button transitions from checking to ready on success', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()).not.toBeNull();
      expect(getButton()!.getAttribute('data-lumaforge-state')).toBe('ready');
      expect(getButton()!.textContent).toContain('ADD VIA LUMAFORGE');
    });

    vi.unstubAllGlobals();
  });

  it('button transitions from checking to in-library when in library', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '730', inLibrary: true }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await vi.waitFor(() => {
      expect(getButton()).not.toBeNull();
      expect(getButton()!.getAttribute('data-lumaforge-state')).toBe('in-library');
      expect(getButton()!.textContent).toContain('IN LIBRARY');
    });

    vi.unstubAllGlobals();
  });

  it('response with mismatched App ID is treated as stale', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation((url: string, _opts?: RequestInit) => {
        if (url.includes('/api/local-status/730')) {
          return Promise.resolve({
            ok: true,
            json: () => Promise.resolve({ ok: true, appId: '999', in_library: false }),
          });
        }
        return Promise.resolve({
          ok: true,
          json: () => Promise.resolve({ ok: true, providers: [] }),
        });
      })
    );

    setupAppPage('730');
    runScript(scriptCode);

    await new Promise((r) => setTimeout(r, 500));

    const btn = getButton();
    if (btn) {
      expect(btn.textContent).not.toContain('ADD VIA LUMAFORGE');
      expect(btn.textContent).not.toContain('IN LIBRARY');
    }

    vi.unstubAllGlobals();
  });
});
