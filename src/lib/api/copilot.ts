/**
 * GitHub Copilot OAuth API
 *
 * Provides API functions related to the GitHub Copilot OAuth device code process.
 * Supports multiple account management.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * GitHub account information (public information)
 */
export interface GitHubAccount {
  /** GitHub user ID (unique identifier) */
  id: string;
  /** GitHub username */
  login: string;
  /** Avatar URL */
  avatar_url: string | null;
  /** Authentication timestamp (Unix seconds) */
  authenticated_at: number;
  /** GitHub domain name (github.com or GHES domain name) */
  github_domain: string;
}

/**
 * Copilot available models
 */
export interface CopilotModel {
  id: string;
  name: string;
  vendor: string;
  model_picker_enabled: boolean;
  model_type?: string;
  policy_state?: string;
  context_window?: number;
  max_context_window_tokens?: number;
  max_output_tokens?: number;
  supports_tool_calls?: boolean;
  supported_endpoints?: string[];
  supports_parallel_tool_calls?: boolean;
  supports_vision?: boolean;
  reasoning_efforts?: string[];
}

/**
 * Get a list of available models for Copilot
 *
 * @returns list of available models
 */
export async function copilotGetModels(): Promise<CopilotModel[]> {
  return invoke<CopilotModel[]>("copilot_get_models");
}

export async function copilotOpenModelCatalog(): Promise<void> {
  return invoke("open_generated_model_catalog");
}

/**
 * Quota details
 */
interface QuotaDetail {
  entitlement: number;
  remaining: number;
  percent_remaining: number;
  unlimited: boolean;
}

/**
 * Quota Snapshot
 */
interface QuotaSnapshots {
  chat: QuotaDetail;
  completions: QuotaDetail;
  premium_interactions: QuotaDetail;
}

/**
 * Copilot usage response
 */
interface CopilotUsageResponse {
  copilot_plan: string;
  quota_reset_date: string;
  quota_snapshots: QuotaSnapshots;
}

/**
 * Get Copilot usage information
 *
 * @returns usage information, including plan type, reset date, and quota snapshot
 */
export async function copilotGetUsage(): Promise<CopilotUsageResponse> {
  return invoke<CopilotUsageResponse>("copilot_get_usage");
}

/**
 * Get the list of Copilot available models for the specified account
 *
 * @param accountId - GitHub user ID
 * @returns list of available models
 */
export async function copilotGetModelsForAccount(
  accountId: string,
): Promise<CopilotModel[]> {
  return invoke<CopilotModel[]>("copilot_get_models_for_account", {
    accountId,
  });
}

/**
 * Get Copilot usage information for a specified account
 *
 * @param accountId - GitHub user ID
 * @returns usage information
 */
export async function copilotGetUsageForAccount(
  accountId: string,
): Promise<CopilotUsageResponse> {
  return invoke<CopilotUsageResponse>("copilot_get_usage_for_account", {
    accountId,
  });
}
