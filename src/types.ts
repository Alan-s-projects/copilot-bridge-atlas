export interface Provider {
  id: string;
  name: string;
  settingsConfig: Record<string, any>;
  meta?: ProviderMeta;
}

interface AuthBinding {
  authProvider?: string;
  accountId?: string;
}

export interface ProviderMeta {
  authBinding?: AuthBinding;
  providerType?: string;
}

export interface CodexCatalogModel {
  model: string;
  displayName?: string;
  /** A missing preference enables a model by default. */
  enabled?: boolean;
  available?: boolean;
  vendor?: string;
  contextWindow?: string | number;
  maxContextWindow?: number;
  maxOutputTokens?: number;
  supportsToolCalls?: boolean;
  supportsParallelToolCalls?: boolean;
  inputModalities?: string[];
  baseInstructions?: string;
  reasoningLevels?: string[];
  supportedReasoningLevels?: string[];
  defaultReasoningLevel?: string;
}

// Application preferences belong to ~/.copilot-bridge-atlas/settings.json.
export interface Settings {
  launchOnStartup?: boolean;
  usageDashboardRefreshIntervalMs?: number;
  currentProviderCodex?: string;
  backupIntervalHours?: number;
  backupRetainCount?: number;
}
