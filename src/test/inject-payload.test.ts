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
  // Use Function() to execute in the vitest global scope so window.__lumaforge_ssh__ is accessible
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
  document.querySelectorAll('style[data-lumaforge-style]').forEach((el) => el.remove());
  delete (window as Record<string, unknown>)['__lumaforge_ssh__'];
  history.replaceState(null, '', '/');
}

declare global {
  interface Window {
    __lumaforge_ssh__?: {
      activate: () => void;
      deactivate: () => void;
      version: string;
    };
  }
}

describe('steam-store-helper inject.js', () => {
  let scriptCode: string;

  beforeEach(() => {
    scriptCode = loadInjectScript();
    document.body.innerHTML = '';
    cleanup();
  });

  afterEach(() => {
    cleanup();
  });

  it('exposes the lifecycle namespace on window', () => {
    navigateTo('/app/12345');
    runScript(scriptCode);
    expect(window.__lumaforge_ssh__).toBeDefined();
    expect(typeof window.__lumaforge_ssh__!.activate).toBe('function');
    expect(typeof window.__lumaforge_ssh__!.deactivate).toBe('function');
    expect(window.__lumaforge_ssh__!.version).toBe('1.0.0');
  });

  it('creates exactly one button on a valid Steam Store app page', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"><button class="btn_addtocart">Buy</button></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(1);
    expect(buttons[0].getAttribute('data-lumaforge-control')).toBe('download-package');
    expect(buttons[0].getAttribute('data-lumaforge-app-id')).toBe('730');
  });

  it('creates no button on an unsupported URL', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/cart');
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(0);
  });

  it('creates no button on login page', () => {
    navigateTo('/login');
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(0);
  });

  it('rejects malformed IDs (non-numeric in path)', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/abc123');
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(0);
  });

  it('rejects ID of zero', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/0');
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(0);
  });

  it('does not create duplicate buttons on repeated execution', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);
    runScript(scriptCode);

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(1);
  });

  it('idempotent activate() does not duplicate', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);
    window.__lumaforge_ssh__!.activate();
    window.__lumaforge_ssh__!.activate();

    const buttons = document.querySelectorAll('[data-lumaforge-extension="steam-store-helper"]');
    expect(buttons).toHaveLength(1);
  });

  it('button has type="button" and is keyboard accessible', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    expect(btn).not.toBeNull();
    expect(btn.tagName).toBe('BUTTON');
    expect(btn.type).toBe('button');
    expect(btn.getAttribute('aria-label')).toContain('730');
    expect(btn.title).toContain('LumaForge');
  });

  it('button has scoped styles injected', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    const style = document.querySelector('style[data-lumaforge-style]');
    expect(style).not.toBeNull();
    expect(style!.textContent).toContain('data-lumaforge-extension');
  });

  it('deactivation removes the button', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).not.toBeNull();
    window.__lumaforge_ssh__!.deactivate();
    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).toBeNull();
  });

  it('deactivation removes the injected styles', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    expect(document.querySelector('style[data-lumaforge-style]')).not.toBeNull();
    window.__lumaforge_ssh__!.deactivate();
    expect(document.querySelector('style[data-lumaforge-style]')).toBeNull();
  });

  it('repeated deactivate() is safe', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);
    window.__lumaforge_ssh__!.deactivate();
    expect(() => window.__lumaforge_ssh__!.deactivate()).not.toThrow();
  });

  it('click sends GET request to the bridge endpoint', async () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/570');
    runScript(scriptCode);

    const fetchSpy = vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ ok: true, status: 'accepted', appId: '570' }),
    });
    vi.stubGlobal('fetch', fetchSpy);

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    expect(btn).not.toBeNull();
    btn.click();

    await vi.waitFor(() => {
      expect(fetchSpy).toHaveBeenCalledTimes(1);
    });

    const callArgs = fetchSpy.mock.calls[0];
    expect(callArgs[0]).toContain('127.0.0.1:21775');
    expect(callArgs[0]).toContain('/api/download-package/570');
    expect(callArgs[1].method).toBe('GET');
    expect(callArgs[1].mode).toBe('cors');

    vi.unstubAllGlobals();
  });

  it('shows success state after accepted response', async () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        json: () => Promise.resolve({ ok: true, status: 'accepted' }),
      })
    );

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    btn.click();

    await vi.waitFor(() => {
      expect(btn.getAttribute('data-lumaforge-state')).toBe('success');
    });

    vi.unstubAllGlobals();
  });

  it('shows error state on fetch failure', async () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new Error('Network error')));

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    btn.click();

    await vi.waitFor(() => {
      expect(btn.getAttribute('data-lumaforge-state')).toBe('error');
    });

    vi.unstubAllGlobals();
  });

  it('disables button while request is pending', async () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    let resolveFetch!: (v: unknown) => void;
    vi.stubGlobal(
      'fetch',
      vi.fn().mockImplementation(
        () => new Promise((resolve) => { resolveFetch = resolve; })
      )
    );

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    btn.click();

    await new Promise((r) => setTimeout(r, 50));
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute('data-lumaforge-pending')).toBe('true');

    resolveFetch({
      ok: true,
      json: () => Promise.resolve({ ok: true, status: 'accepted' }),
    });
    await vi.waitFor(() => {
      expect(btn.getAttribute('data-lumaforge-pending')).toBeNull();
    });

    vi.unstubAllGlobals();
  });

  it('handles /app/{id}/dlc path format', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730/DLC_Name');
    runScript(scriptCode);

    const btn = document.querySelector('[data-lumaforge-extension="steam-store-helper"]');
    expect(btn).not.toBeNull();
    expect(btn!.getAttribute('data-lumaforge-app-id')).toBe('730');
  });

  it('removes button when navigating to non-app page via activate()', () => {
    document.body.innerHTML = '<div id="game_area_purchase_game"></div>';
    navigateTo('/app/730');
    runScript(scriptCode);
    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).not.toBeNull();

    navigateTo('/cart');
    window.__lumaforge_ssh__!.activate();
    expect(
      document.querySelector('[data-lumaforge-extension="steam-store-helper"]')
    ).toBeNull();
  });

  it('button click does not propagate to parent Steam actions', async () => {
    document.body.innerHTML =
      '<div id="game_area_purchase_game"><button class="btn_addtocart">Buy</button></div>';
    navigateTo('/app/730');
    runScript(scriptCode);

    const parentClickHandler = vi.fn();
    const purchaseArea = document.getElementById('game_area_purchase_game')!;
    purchaseArea.addEventListener('click', parentClickHandler);

    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: () => Promise.resolve({ ok: true, status: 'accepted' }),
    }));

    const btn = document.querySelector(
      '[data-lumaforge-extension="steam-store-helper"]'
    ) as HTMLButtonElement;
    btn.click();

    expect(parentClickHandler).not.toHaveBeenCalled();
    purchaseArea.removeEventListener('click', parentClickHandler);
    vi.unstubAllGlobals();
  });
});
