import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { SettingsView } from '../components/SettingsView';
import type { PluginEntry, SteamRootInfo, AppearanceSettings } from '../types';
import { DEFAULT_APPEARANCE } from '../types';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn().mockResolvedValue({}),
}));

const mockPlugins: PluginEntry[] = [
  {
    id: 'test-ext',
    name: 'Test Extension',
    version: '1.0.0',
    description: 'Test',
    author: 'Test',
    enabled: true,
    source: 'local',
    hasDetect: false,
    scriptPath: null,
    manifestPath: 'manifest.json',
  },
];

const mockSteamRoot: SteamRootInfo = {
  resolvedPath: 'C:\\Program Files (x86)\\Steam',
  isCustom: false,
  configPath: '/test/config.json',
};

describe('SettingsView', () => {
  const defaultProps = {
    plugins: mockPlugins,
    steamRoot: mockSteamRoot,
    onSteamRootUpdated: vi.fn(),
    appearance: DEFAULT_APPEARANCE,
    onAppearanceUpdated: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders settings title and description', () => {
    render(<SettingsView {...defaultProps} />);
    expect(screen.getByText('Settings')).toBeInTheDocument();
    expect(screen.getByText('Configure LumaForge Lite to match your workflow.')).toBeInTheDocument();
  });

  it('renders all six category rows', () => {
    render(<SettingsView {...defaultProps} />);
    expect(screen.getByText('General')).toBeInTheDocument();
    expect(screen.getByText('Appearance')).toBeInTheDocument();
    expect(screen.getByText('Extensions')).toBeInTheDocument();
    expect(screen.getByText('Integrations')).toBeInTheDocument();
    expect(screen.getByText('Advanced')).toBeInTheDocument();
    expect(screen.getByText('About')).toBeInTheDocument();
  });

  it('shows extension count badge for Extensions category', () => {
    render(<SettingsView {...defaultProps} />);
    expect(screen.getByText('1')).toBeInTheDocument();
  });

  it('shows "Active" badge for Integrations when steam root exists', () => {
    render(<SettingsView {...defaultProps} />);
    expect(screen.getByText('Active')).toBeInTheDocument();
  });

  it('does not show "Active" badge when steam root is missing', () => {
    render(<SettingsView {...defaultProps} steamRoot={null} />);
    const activeBadge = screen.queryByText('Active');
    expect(activeBadge).not.toBeInTheDocument();
  });

  it('navigates to General sub-screen when General row is clicked', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('General'));
    expect(screen.getByText('Application preferences and common settings.')).toBeInTheDocument();
    expect(screen.getByText('Plugins Directory')).toBeInTheDocument();
  });

  it('navigates to About sub-screen when About row is clicked', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('About'));
    expect(screen.getByText('LumaForge Lite')).toBeInTheDocument();
    expect(screen.getByText('Tauri v2 + React')).toBeInTheDocument();
  });

  it('navigates to Extensions sub-screen and shows plugin list', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('Extensions'));
    expect(screen.getByText('Total Extensions')).toBeInTheDocument();
    expect(screen.getByText('Test Extension')).toBeInTheDocument();
  });

  it('navigates to Integrations sub-screen and shows steam path', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('Integrations'));
    expect(screen.getByText('Steam Root Path')).toBeInTheDocument();
  });

  it('navigates to Advanced sub-screen', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('Advanced'));
    expect(screen.getByText('Clear Engine Cache')).toBeInTheDocument();
    expect(screen.getByText('Rescan Plugins')).toBeInTheDocument();
  });

  it('navigates to Appearance sub-screen', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('Appearance'));
    expect(screen.getByText('Theme')).toBeInTheDocument();
    expect(screen.getByText('Midnight Blue')).toBeInTheDocument();
  });

  it('shows breadcrumb with back button in sub-screens', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('General'));
    expect(screen.getByLabelText('Back to Settings')).toBeInTheDocument();
    expect(screen.getByText('Settings')).toBeInTheDocument();
  });

  it('navigates back to home when back button is clicked', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('General'));
    expect(screen.getByText('Application preferences and common settings.')).toBeInTheDocument();
    await user.click(screen.getByLabelText('Back to Settings'));
    expect(screen.getByText('Configure LumaForge Lite to match your workflow.')).toBeInTheDocument();
  });

  it('shows empty extensions state when no plugins', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} plugins={[]} />);
    await user.click(screen.getByText('Extensions'));
    expect(screen.getByText('No extensions installed')).toBeInTheDocument();
  });

  it('General sub-screen shows correct extension counts', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('General'));
    expect(screen.getByText('1 enabled of 1 total')).toBeInTheDocument();
  });

  it('Integrations sub-screen shows auto-detected badge', async () => {
    const user = userEvent.setup();
    render(<SettingsView {...defaultProps} />);
    await user.click(screen.getByText('Integrations'));
    expect(screen.getByText('auto-detected')).toBeInTheDocument();
  });
});
