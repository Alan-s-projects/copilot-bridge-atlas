export interface Provider {
  id: string;
  name: string;
  settingsConfig: Record<string, any>;
  meta?: ProviderMeta;
}

interface AuthBinding {
  source: "provider_config" | "managed_account";
  authProvider?: string;
  accountId?: string;
}

export interface ProviderMeta {
  apiFormat?: "openai_chat" | "openai_responses";
  // Retired metadata is accepted from old records but never controls routing.
  codexCopilotApiFormat?: string;
  authBinding?: AuthBinding;
  providerType?: string;
  // Accept the saved account binding used by earlier provider records.
  githubAccountId?: string;
}

export interface CodexCatalogModel {
  model: string;
  displayName?: string;
  /** Defaults to enabled for catalog rows saved by earlier app versions. */
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
