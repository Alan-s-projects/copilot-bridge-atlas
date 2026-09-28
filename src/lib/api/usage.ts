import { invoke } from "@tauri-apps/api/core";
import type {
  UsageSummary,
  UsageTrends,
  TrendGrouping,
  ModelStats,
  UnpricedModelUsage,
  LogFilters,
  ModelPricing,
  PaginatedLogs,
  RequestDiagnosticDetail,
} from "@/types/usage";

export const usageApi = {
  // Proxy usage statistics methods
  getUsageSummary: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
    providerName?: string,
    model?: string,
  ): Promise<UsageSummary> => {
    return invoke("get_usage_summary", {
      startDate,
      endDate,
      appType,
      providerName,
      model,
    });
  },

  getUsageTrends: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
    providerName?: string,
    model?: string,
    grouping?: TrendGrouping,
  ): Promise<UsageTrends> => {
    return invoke("get_usage_trends", {
      startDate,
      endDate,
      appType,
      providerName,
      model,
      grouping,
    });
  },

  getModelStats: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
    providerName?: string,
    model?: string,
  ): Promise<ModelStats[]> => {
    return invoke("get_model_stats", {
      startDate,
      endDate,
      appType,
      providerName,
      model,
    });
  },

  getUnpricedModelUsage: async (
    startDate?: number,
    endDate?: number,
    appType?: string,
    providerName?: string,
    model?: string,
  ): Promise<UnpricedModelUsage[]> => {
    return invoke("get_unpriced_model_usage", {
      startDate,
      endDate,
      appType,
      providerName,
      model,
    });
  },

  getRequestLogs: async (
    filters: LogFilters,
    page: number = 0,
    pageSize: number = 20,
  ): Promise<PaginatedLogs> => {
    return invoke("get_request_logs", {
      filters,
      page,
      pageSize,
    });
  },

  getRequestDiagnostics: async (
    requestId: string,
  ): Promise<RequestDiagnosticDetail | null> =>
    invoke("get_request_diagnostics", { requestId }),

  getModelPricing: async (): Promise<ModelPricing[]> => {
    return invoke("get_model_pricing");
  },

  updateModelPricing: async (
    modelId: string,
    displayName: string,
    inputCost: string,
    outputCost: string,
    cacheReadCost: string,
    cacheCreationCost: string,
    longContext?: ModelPricing["longContext"],
  ): Promise<void> => {
    return invoke("update_model_pricing", {
      modelId,
      displayName,
      inputCost,
      outputCost,
      cacheReadCost,
      cacheCreationCost,
      longContext,
    });
  },

  deleteModelPricing: async (modelId: string): Promise<void> => {
    return invoke("delete_model_pricing", { modelId });
  },

  resetModelPricingToDefaults: async (): Promise<void> => {
    return invoke("reset_model_pricing_to_defaults");
  },
};
