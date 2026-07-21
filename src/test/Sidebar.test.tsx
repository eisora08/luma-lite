import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Sidebar } from '../components/Sidebar';

const defaultProps = {
  activeView: 'dashboard' as const,
  onViewChange: vi.fn(),
  pluginCount: 0,
  expanded: false,
  onToggleExpand: vi.fn(),
};

describe('Sidebar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the LumaForge logo', () => {
    render(<Sidebar {...defaultProps} />);
    expect(screen.getByText('L')).toBeInTheDocument();
  });

  it('shows expand tooltip when collapsed', () => {
    render(<Sidebar {...defaultProps} />);
    const btn = screen.getByRole('button', { name: 'Expand navigation' });
    expect(btn).toHaveAttribute('data-tooltip', 'Expand navigation');
  });

  it('shows collapse tooltip when expanded', () => {
    render(<Sidebar {...defaultProps} expanded={true} />);
    const btn = screen.getByRole('button', { name: 'Collapse navigation' });
    expect(btn).toHaveAttribute('data-tooltip', 'Collapse navigation');
  });

  it('calls onToggleExpand when logo button is clicked', async () => {
    const user = userEvent.setup();
    const onToggle = vi.fn();
    render(<Sidebar {...defaultProps} onToggleExpand={onToggle} />);
    await user.click(screen.getByRole('button', { name: 'Expand navigation' }));
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('has aria-expanded on logo button', () => {
    render(<Sidebar {...defaultProps} />);
    const btn = screen.getByRole('button', { name: 'Expand navigation' });
    expect(btn).toHaveAttribute('aria-expanded', 'false');
  });

  it('has aria-expanded=true when expanded', () => {
    render(<Sidebar {...defaultProps} expanded={true} />);
    const btn = screen.getByRole('button', { name: 'Collapse navigation' });
    expect(btn).toHaveAttribute('aria-expanded', 'true');
  });

  it('has aria-controls referencing sidebar-nav', () => {
    render(<Sidebar {...defaultProps} />);
    const btn = screen.getByRole('button', { name: 'Expand navigation' });
    expect(btn).toHaveAttribute('aria-controls', 'sidebar-nav');
  });

  it('renders all nav items', () => {
    render(<Sidebar {...defaultProps} />);
    expect(screen.getByText('Dashboard')).toBeInTheDocument();
    expect(screen.getByText('Extensions')).toBeInTheDocument();
    expect(screen.getByText('Settings')).toBeInTheDocument();
  });

  it('marks the active nav item', () => {
    render(<Sidebar {...defaultProps} activeView="extensions" />);
    const ext = screen.getByText('Extensions').closest('button');
    expect(ext).toHaveAttribute('aria-current', 'page');
  });

  it('shows plugin count badge when count > 0', () => {
    render(<Sidebar {...defaultProps} pluginCount={5} />);
    expect(screen.getByText('5')).toBeInTheDocument();
  });

  it('does not show badge when count is 0', () => {
    render(<Sidebar {...defaultProps} pluginCount={0} />);
    expect(screen.queryByText('0')).not.toBeInTheDocument();
  });

  it('shows LumaForge text when expanded', () => {
    render(<Sidebar {...defaultProps} expanded={true} />);
    expect(screen.getByText('LumaForge')).toBeInTheDocument();
  });

  it('does not show LumaForge text when collapsed', () => {
    render(<Sidebar {...defaultProps} expanded={false} />);
    expect(screen.queryByText('LumaForge')).not.toBeInTheDocument();
  });

  it('has no separate collapse button to the right', () => {
    render(<Sidebar {...defaultProps} expanded={true} />);
    const collapseBtn = screen.queryByRole('button', { name: 'Collapse navigation' });
    expect(collapseBtn).toBeInTheDocument();
    const expandBtn = screen.queryByRole('button', { name: 'Expand navigation' });
    expect(expandBtn).not.toBeInTheDocument();
  });

  it('calls onViewChange when a nav item is clicked', async () => {
    const user = userEvent.setup();
    const onViewChange = vi.fn();
    render(<Sidebar {...defaultProps} onViewChange={onViewChange} />);
    await user.click(screen.getByText('Extensions'));
    expect(onViewChange).toHaveBeenCalledWith('extensions');
  });
});
