import type { AppearanceSettings, ThemeId, SurfaceStyle, DensityLevel } from '../types';

interface AppearanceViewProps {
  settings: AppearanceSettings;
  onUpdate: (settings: AppearanceSettings) => void;
  onBack: () => void;
}

const THEMES: { id: ThemeId; label: string; desc: string; swatches: string[] }[] = [
  { id: 'midnight-blue', label: 'Midnight Blue', desc: 'Deep navy with blue accent — the default LumaForge identity', swatches: ['#070b14', '#0c1d3d', '#60a5fa'] },
  { id: 'oled-black', label: 'OLED Black', desc: 'True black surfaces optimized for OLED displays', swatches: ['#000000', '#0a0a0a', '#60a5fa'] },
  { id: 'steam-gray', label: 'Steam Gray', desc: 'Familiar dark blue-gray inspired by the Steam client', swatches: ['#1b2838', '#2a475e', '#66c0ff'] },
  { id: 'crimson-dark', label: 'Crimson Dark', desc: 'Deep dark with pink accent for a bold aesthetic', swatches: ['#0f0810', '#1f1424', '#f472b6'] },
];

const SURFACES: { id: SurfaceStyle; label: string; desc: string }[] = [
  { id: 'solid', label: 'Solid', desc: 'Opaque surfaces with highest readability' },
  { id: 'tinted', label: 'Tinted', desc: 'Surfaces subtly influenced by the theme color' },
  { id: 'glass', label: 'Liquid Glass', desc: 'Restrained transparency with backdrop blur' },
];

const DENSITIES: { id: DensityLevel; label: string; desc: string }[] = [
  { id: 'comfortable', label: 'Comfortable', desc: 'Standard padding and spacing' },
  { id: 'compact', label: 'Compact', desc: 'Reduced padding for more content density' },
];

function ChevronRight() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="6 4 10 8 6 12" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 8 6.5 11.5 13 4.5" />
    </svg>
  );
}

export function AppearanceView({ settings, onUpdate, onBack }: AppearanceViewProps) {
  const update = (partial: Partial<AppearanceSettings>) => {
    onUpdate({ ...settings, ...partial });
  };

  return (
    <div className="view-content">
      <nav className="settings-breadcrumb" aria-label="Breadcrumb">
        <button className="settings-back-btn" onClick={onBack} aria-label="Back to Settings">
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="10 3 5 8 10 13" />
          </svg>
        </button>
        <button className="settings-breadcrumb-link" onClick={onBack}>Settings</button>
        <span className="settings-breadcrumb-sep"><ChevronRight /></span>
        <span className="settings-breadcrumb-current">Appearance</span>
      </nav>

      <div className="settings-category-header">
        <h2 className="settings-category-title">Appearance</h2>
        <p className="settings-category-desc">Customize LumaForge's theme, surfaces, visual density, and visual preferences.</p>
      </div>

      <div className="settings-sections">
        {/* Theme */}
        <div className="settings-section-group">
          <h3 className="settings-section-label">Theme</h3>
          <div className="theme-grid">
            {THEMES.map((t) => (
              <button
                key={t.id}
                className={`theme-card ${settings.theme === t.id ? 'theme-card-active' : ''}`}
                onClick={() => update({ theme: t.id })}
                role="radio"
                aria-checked={settings.theme === t.id}
                aria-label={t.label}
              >
                <div className="theme-card-swatches">
                  {t.swatches.map((c, i) => (
                    <span key={i} className="theme-swatch" style={{ background: c }} />
                  ))}
                </div>
                <div className="theme-card-info">
                  <span className="theme-card-name">{t.label}</span>
                  <span className="theme-card-desc">{t.desc}</span>
                </div>
                {settings.theme === t.id && (
                  <span className="theme-card-check"><CheckIcon /></span>
                )}
              </button>
            ))}
          </div>
        </div>

        {/* Surface Style */}
        <div className="settings-section-group">
          <h3 className="settings-section-label">Surface Style</h3>
          <div className="surface-options">
            {SURFACES.map((s) => (
              <button
                key={s.id}
                className={`surface-card ${settings.surfaceStyle === s.id ? 'surface-card-active' : ''}`}
                onClick={() => update({ surfaceStyle: s.id })}
                role="radio"
                aria-checked={settings.surfaceStyle === s.id}
                aria-label={s.label}
              >
                <div className="surface-card-preview" data-surface-preview={s.id} />
                <div className="surface-card-info">
                  <span className="surface-card-name">{s.label}</span>
                  <span className="surface-card-desc">{s.desc}</span>
                </div>
                {settings.surfaceStyle === s.id && (
                  <span className="surface-card-check"><CheckIcon /></span>
                )}
              </button>
            ))}
          </div>
        </div>

        {/* Interface Density */}
        <div className="settings-section-group">
          <h3 className="settings-section-label">Interface Density</h3>
          <div className="density-options">
            {DENSITIES.map((d) => (
              <button
                key={d.id}
                className={`density-card ${settings.density === d.id ? 'density-card-active' : ''}`}
                onClick={() => update({ density: d.id })}
                role="radio"
                aria-checked={settings.density === d.id}
                aria-label={d.label}
              >
                <div className="density-card-info">
                  <span className="density-card-name">{d.label}</span>
                  <span className="density-card-desc">{d.desc}</span>
                </div>
                {settings.density === d.id && (
                  <span className="density-card-check"><CheckIcon /></span>
                )}
              </button>
            ))}
          </div>
        </div>

        {/* Motion and Effects */}
        <div className="settings-section-group">
          <h3 className="settings-section-label">Motion and Effects</h3>
          <div className="settings-group">
            <div className="settings-row">
              <div className="settings-row-left">
                <span className="settings-row-label">Reduce Motion</span>
                <span className="settings-row-desc">Minimize animations and transitions throughout the interface</span>
              </div>
              <div className="settings-row-right">
                <button
                  className={`toggle ${settings.reduceMotion ? 'toggle-on' : ''}`}
                  role="switch"
                  aria-checked={settings.reduceMotion}
                  aria-label="Reduce motion"
                  onClick={() => update({ reduceMotion: !settings.reduceMotion })}
                >
                  <span className="toggle-track" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
