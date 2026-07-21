import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ExtensionsView } from '../components/ExtensionsView';
import type { PluginEntry } from '../types';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const mockPlugins: PluginEntry[] = [
  {
    id: 'steam-store-helper',
    name: 'Steam Store Helper',
    version: '1.0.0',
    description: 'Adds download button to Steam store pages',
    author: 'LumaForge',
    enabled: true,
    source: 'local',
    hasDetect: true,
    scriptPath: 'extension.lua',
    manifestPath: 'manifest.json',
  },
  {
    id: 'placeholder',
    name: 'Placeholder',
    version: '0.1.0',
    description: 'A minimal test extension',
    author: '',
    enabled: false,
    source: 'local',
    hasDetect: false,
    scriptPath: null,
    manifestPath: 'manifest.json',
  },
];

describe('ExtensionsView', () => {
  const defaultProps = {
    plugins: mockPlugins,
    loading: false,
    onToggle: vi.fn().mockResolvedValue(true),
    onReload: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders all extension cards', () => {
    render(<ExtensionsView {...defaultProps} />);
    expect(screen.getByText('Steam Store Helper')).toBeInTheDocument();
    expect(screen.getByText('Placeholder')).toBeInTheDocument();
  });

  it('shows extension count in description', () => {
    render(<ExtensionsView {...defaultProps} />);
    expect(screen.getByText('1 of 2 extensions enabled.')).toBeInTheDocument();
  });

  it('shows toggle buttons for each extension', () => {
    render(<ExtensionsView {...defaultProps} />);
    const toggles = screen.getAllByRole('switch');
    expect(toggles).toHaveLength(2);
  });

  it('sets aria-checked on toggle based on enabled state', () => {
    render(<ExtensionsView {...defaultProps} />);
    const toggles = screen.getAllByRole('switch');
    expect(toggles[0]).toHaveAttribute('aria-checked', 'true');
    expect(toggles[1]).toHaveAttribute('aria-checked', 'false');
  });

  it('calls onToggle with correct plugin when toggle is clicked', async () => {
    const user = userEvent.setup();
    render(<ExtensionsView {...defaultProps} />);
    const toggles = screen.getAllByRole('switch');
    await user.click(toggles[0]);
    expect(defaultProps.onToggle).toHaveBeenCalledWith(mockPlugins[0]);
  });

  it('disables toggle while pending', async () => {
    let resolveToggle!: (value: boolean) => Promise<boolean>;
    const slowToggle = vi.fn().mockImplementation(
      () => new Promise<boolean>((resolve) => { resolveToggle = resolve; })
    );

    const { rerender } = render(
      <ExtensionsView {...defaultProps} onToggle={slowToggle} />
    );

    const toggles = screen.getAllByRole('switch');
    await userEvent.setup().click(toggles[0]);

    // Toggle should be disabled during pending
    const updatedToggles = screen.getAllByRole('switch');
    expect(updatedToggles[0]).toBeDisabled();
    expect(updatedToggles[0]).toHaveClass('toggle-loading');

    // Resolve the toggle
    await resolveToggle(true);
  });

  it('shows loading state when loading prop is true', () => {
    render(<ExtensionsView {...defaultProps} loading={true} />);
    expect(screen.getByText('Scanning plugins...')).toBeInTheDocument();
  });

  it('shows empty state when no plugins', () => {
    render(<ExtensionsView {...defaultProps} plugins={[]} />);
    expect(screen.getByText('No extensions found')).toBeInTheDocument();
  });

  it('shows error feedback when toggle fails', async () => {
    const failToggle = vi.fn().mockResolvedValue(false);
    const user = userEvent.setup();
    render(<ExtensionsView {...defaultProps} onToggle={failToggle} />);

    const toggles = screen.getAllByRole('switch');
    await user.click(toggles[0]);

    // Wait for error state
    await vi.waitFor(() => {
      expect(screen.getByText(/Failed to disable/)).toBeInTheDocument();
    });
  });

  it('version and author are displayed', () => {
    render(<ExtensionsView {...defaultProps} />);
    expect(screen.getByText(/v1\.0\.0/)).toBeInTheDocument();
    expect(screen.getByText(/· LumaForge/)).toBeInTheDocument();
  });
});
