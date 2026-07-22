export interface PluginEntry {
  id: string;
  name: string;
  version: string;
  description: string;
  author: string;
  enabled: boolean;
  source: string;
  hasDetect: boolean;
  scriptPath: string | null;
  manifestPath: string | null;
  cefInjection?: boolean;
  injectScript?: string;
  targetUrl?: string;
}

export interface SteamRootInfo {
  resolvedPath: string | null;
  isCustom: boolean;
  configPath: string;
}

export interface BridgeStatus {
  running: boolean;
  port: number;
}

export type ViewId = 'dashboard' | 'extensions' | 'settings';

export type SettingsCategoryId = 'general' | 'appearance' | 'extensions' | 'integrations' | 'downloads' | 'advanced' | 'about';

export interface SettingsCategory {
  id: SettingsCategoryId;
  label: string;
  description: string;
}

export type SettingsSubView = SettingsCategoryId | null;

export type ThemeId = 'midnight-blue' | 'oled-black' | 'steam-gray' | 'crimson-dark';

export type SurfaceStyle = 'solid' | 'tinted' | 'glass';

export type DensityLevel = 'comfortable' | 'compact';

export interface AppearanceSettings {
  theme: ThemeId;
  surfaceStyle: SurfaceStyle;
  density: DensityLevel;
  reduceMotion: boolean;
}

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  theme: 'midnight-blue',
  surfaceStyle: 'tinted',
  density: 'comfortable',
  reduceMotion: false,
};

export interface InjectionStatus {
  targetUrl: string;
  injectedTabs: number;
  injectedUrls: string[];
}

export interface ProviderConfig {
  id: string;
  name: string;
  enabled: boolean;
  baseUrl: string;
  apiKey?: string;
  hasApiKey?: boolean;
  keyPreview?: string;
  adapterAvailable?: boolean;
}

export interface ProviderCapability {
  label: string;
  color: 'green' | 'blue' | 'yellow' | 'gray';
}

export interface ProviderDef {
  id: string;
  name: string;
  description: string;
  capabilities: ProviderCapability[];
  supportedTypes: string[];
  requiresApiKey: boolean;
}

export interface RepositorySource {
  url: string;
  label?: string;
  lastFetched?: number;
  lastError?: string;
}

export interface RepositoryExtensionView {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  categories: string[];
  manifestUrl: string;
  verified: boolean;
  installed: boolean;
  installedVersion?: string;
  repositoryUrl: string;
  repositoryLabel?: string;
}

export interface ListRepositoriesResult {
  repositories: RepositorySource[];
}

export interface ListRepositoryExtensionsResult {
  extensions: RepositoryExtensionView[];
  repositories: RepositorySource[];
}
