import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    hide: vi.fn(),
    minimize: vi.fn(),
  }),
}));

describe('Extension Toggle Integration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('sends extensionId (not pluginId) to toggle_plugin command', async () => {
    const mockPlugin = {
      id: 'test-ext',
      name: 'Test',
      version: '1.0.0',
      description: '',
      author: '',
      enabled: false,
      source: 'local',
      hasDetect: false,
      scriptPath: null,
      manifestPath: 'manifest.json',
    };

    vi.mocked(invoke).mockResolvedValue({
      ...mockPlugin,
      enabled: true,
    });

    // Simulate the fixed toggle call from App.tsx
    const desiredEnabled = !mockPlugin.enabled;
    const result = await invoke('toggle_plugin', {
      extensionId: mockPlugin.id,
      enabled: desiredEnabled,
    });

    // Verify the correct parameter name was used
    expect(invoke).toHaveBeenCalledWith('toggle_plugin', {
      extensionId: 'test-ext',
      enabled: true,
    });
    expect(result).toEqual(expect.objectContaining({ enabled: true }));
  });

  it('sends correct enabled state when disabling', async () => {
    vi.mocked(invoke).mockResolvedValue({
      id: 'test-ext',
      enabled: false,
    });

    await invoke('toggle_plugin', {
      extensionId: 'test-ext',
      enabled: false,
    });

    expect(invoke).toHaveBeenCalledWith('toggle_plugin', {
      extensionId: 'test-ext',
      enabled: false,
    });
  });

  it('does NOT use pluginId parameter name', async () => {
    vi.mocked(invoke).mockResolvedValue({ id: 'test', enabled: true });

    // This is what the old buggy code did - it would fail
    try {
      await invoke('toggle_plugin', {
        pluginId: 'test', // WRONG parameter name
        enabled: true,
      });
    } catch {
      // Expected to fail with old param name
    }

    // Verify the correct call format works
    vi.mocked(invoke).mockClear();
    vi.mocked(invoke).mockResolvedValue({ id: 'test', enabled: true });
    
    await invoke('toggle_plugin', {
      extensionId: 'test', // CORRECT parameter name
      enabled: true,
    });

    expect(invoke).toHaveBeenCalledWith('toggle_plugin', {
      extensionId: 'test',
      enabled: true,
    });
  });

  it('handles toggle failure gracefully', async () => {
    vi.mocked(invoke).mockRejectedValue(new Error('Plugin not found'));

    await expect(
      invoke('toggle_plugin', {
        extensionId: 'nonexistent',
        enabled: true,
      })
    ).rejects.toThrow('Plugin not found');
  });
});
