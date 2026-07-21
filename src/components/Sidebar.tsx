import type { ViewId } from '../types';

interface SidebarProps {
  activeView: ViewId;
  onViewChange: (view: ViewId) => void;
  pluginCount: number;
  expanded: boolean;
  onToggleExpand: () => void;
}

const PanelLeftOpen = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
    <rect x="1.5" y="1.5" width="13" height="13" rx="2" />
    <line x1="7" y1="4.5" x2="7" y2="11.5" />
    <line x1="10" y1="7.5" x2="11.5" y2="6" />
    <line x1="10" y1="7.5" x2="11.5" y2="9" />
  </svg>
);

const PanelLeftClose = (
  <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
    <rect x="1.5" y="1.5" width="13" height="13" rx="2" />
    <line x1="9" y1="4.5" x2="9" y2="11.5" />
    <line x1="6" y1="7.5" x2="4.5" y2="6" />
    <line x1="6" y1="7.5" x2="4.5" y2="9" />
  </svg>
);

const NavIcon = ({ viewId }: { viewId: ViewId }) => {
  const map: Record<ViewId, JSX.Element> = {
    dashboard: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <rect x="2" y="2" width="7" height="7" rx="1.5" />
        <rect x="11" y="2" width="7" height="4" rx="1.5" />
        <rect x="2" y="11" width="7" height="7" rx="1.5" />
        <rect x="11" y="8" width="7" height="10" rx="1.5" />
      </svg>
    ),
    extensions: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
        <path d="M10 2v4M10 14v4M2 10h4M14 10h4" />
        <circle cx="10" cy="10" r="2.5" />
      </svg>
    ),
    settings: (
      <svg viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="10" cy="10" r="2.5" />
        <path d="M17.4 10c0-.5-.3-1-.8-1.2l-1-.4c-.1-.4-.3-.8-.5-1.1l.3-1c.3-.5.2-1.1-.2-1.5l-.7-.7c-.4-.4-1-.5-1.5-.2l-1 .3c-.4-.2-.8-.4-1.1-.5l-.4-1c-.3-.5-.8-.8-1.2-.8s-1 .3-1.2.8l-.4 1c-.4.1-.8.3-1.1.5l-1-.3c-.5-.3-1.1-.2-1.5.2l-.7.7c-.4.4-.5 1-.2 1.5l.3 1c-.2.4-.4.8-.5 1.1l-1 .4c-.5.3-.8.7-.8 1.2s.3 1 .8 1.2l1 .4c.1.4.3.8.5 1.1l-.3 1c-.3.5-.2 1.1.2 1.5l.7.7c.4.4 1 .5 1.5.2l1-.3c.4.2.8.4 1.1.5l.4 1c.3.5.8.8 1.2.8s1-.3 1.2-.8l.4-1c.4-.1.8-.3 1.1-.5l1 .3c.5.3 1.1.2 1.5-.2l.7-.7c.4-.4.5-1 .2-1.5l-.3-1c.2-.4.4-.8.5-1.1l1-.4c.5-.3.8-.7.8-1.2z" />
      </svg>
    ),
  };
  return map[viewId] ?? map.dashboard;
};

const NAV_ITEMS: { id: ViewId; label: string }[] = [
  { id: 'dashboard', label: 'Dashboard' },
  { id: 'extensions', label: 'Extensions' },
  { id: 'settings', label: 'Settings' },
];

export function Sidebar({ activeView, onViewChange, pluginCount, expanded, onToggleExpand }: SidebarProps) {
  return (
    <nav
      className={`sidebar ${expanded ? 'sidebar-expanded' : ''}`}
      aria-label="Main navigation"
      id="sidebar-nav"
    >
      <div className="sidebar-brand" data-tauri-drag-region>
        <button
          className="sidebar-logo-btn"
          data-tauri-drag-region="noDrag"
          onClick={onToggleExpand}
          aria-label={expanded ? 'Collapse navigation' : 'Expand navigation'}
          aria-expanded={expanded}
          aria-controls="sidebar-nav"
          data-tooltip={expanded ? 'Collapse navigation' : 'Expand navigation'}
        >
          <span className="sidebar-logo-layer sidebar-logo-mark">
            <span className="sidebar-logo-l">L</span>
          </span>
          <span className="sidebar-logo-layer sidebar-logo-icon">
            {expanded ? PanelLeftClose : PanelLeftOpen}
          </span>
        </button>
        {expanded && <span className="sidebar-brand-text">LumaForge</span>}
      </div>

      <div className="sidebar-nav-items" role="list">
        {NAV_ITEMS.map((item) => (
          <button
            key={item.id}
            className={`sidebar-item ${activeView === item.id ? 'sidebar-item-active' : ''}`}
            onClick={() => onViewChange(item.id)}
            aria-current={activeView === item.id ? 'page' : undefined}
            aria-label={item.label}
            data-tooltip={item.label}
            role="listitem"
          >
            <span className="sidebar-icon">
              <NavIcon viewId={item.id} />
            </span>
            <span className="sidebar-label">{item.label}</span>
            {item.id === 'extensions' && pluginCount > 0 && (
              <span className="sidebar-badge">{pluginCount}</span>
            )}
          </button>
        ))}
      </div>
    </nav>
  );
}
