export interface RequestLog {
  requestId: string;
  providerId: string;
  providerName?: string;
  appType: string;
  model: string;
  requestModel?: string;
  requestedReasoningEffort?: string;
  appliedReasoningEffort?: string;
  /** 写入时实际用于计价的模型名；路由接管 + request 计价模式下可能与 model 不同 */
  pricingModel?: string;
  costMultiplier: string;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  inputCostUsd: string;
  outputCostUsd: string;
  cacheReadCostUsd: string;
  cacheCreationCostUsd: string;
  totalCostUsd: string;
  isStreaming: boolean;
  latencyMs: number;
  firstTokenMs?: number;
  durationMs?: number;
  statusCode: number;
  errorMessage?: string;
  createdAt: number;
  dataSource?: string;
}

export interface PaginatedLogs {
  data: RequestLog[];
  total: number;
  page: number;
  pageSize: number;
}

export interface ModelPricing {
  modelId: string;
  displayName: string;
  inputCostPerMillion: string;
  outputCostPerMillion: string;
  cacheReadCostPerMillion: string;
  cacheCreationCostPerMillion: string;
}

export interface UsageSummary {
  totalRequests: number;
  totalCost: string;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
  successRate: number;
  avgLatencyMs: number;
  /** input + output + cache_creation + cache_read, all cache-normalized */
  realTotalTokens: number;
  /** cache_read / (input + cache_creation + cache_read), range 0–1 */
  cacheHitRate: number;
}

export interface DailyStats {
  date: string;
  requestCount: number;
  totalCost: string;
  totalTokens: number;
  totalInputTokens: number;
  totalOutputTokens: number;
  totalCacheCreationTokens: number;
  totalCacheReadTokens: number;
}

export interface ModelStats {
  model: string;
  requestCount: number;
  totalTokens: number;
  totalCost: string;
  avgCostPerRequest: string;
}

export interface UnpricedModelUsage {
  model: string;
  requestCount: number;
  freshInputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheHitRate: number;
}

export interface LogFilters {
  appType?: string;
  providerName?: string;
  model?: string;
  statusCode?: number;
  startDate?: number;
  endDate?: number;
}

/**
 * Dashboard 顶栏的全局筛选维度，作用于 Hero / 趋势图 / 三个统计 Tab。
 *
 * - `providerName` matches the recorded provider name.
 * - `model` 按「有效计价模型」匹配（pricing_model 优先、回落 model，
 *   与模型统计的分组口径一致）。
 */
export interface UsageScopeFilters {
  appType?: string;
  providerName?: string;
  model?: string;
}

export type UsageRangePreset = "today" | "1d" | "7d" | "14d" | "30d" | "custom";

export interface UsageRangeSelection {
  preset: UsageRangePreset;
  customStartDate?: number;
  customEndDate?: number;
  /** When true (custom mode only), endDate resolves to "now" instead of the
   *  fixed customEndDate snapshot, and the end-time field becomes read-only. */
  liveEndTime?: boolean;
}

/** Subset of request-log fields needed to derive cache-normalized input. */
export interface CacheNormalizableLog {
  appType: string;
  inputTokens: number;
  cacheReadTokens: number;
}

/**
 * Codex reports cached tokens inside inputTokens. Preserve already-normalized
 * historical rows when their input value is smaller than the cache count.
 */
export function getFreshInputTokens(log: CacheNormalizableLog): number {
  if (log.appType === "codex" && log.inputTokens >= log.cacheReadTokens) {
    return log.inputTokens - log.cacheReadTokens;
  }
  return log.inputTokens;
}

export const NON_NEGATIVE_DECIMAL_REGEX = /^\d+(?:\.\d+)?$/;

export function isNonNegativeDecimalString(value: string): boolean {
  const trimmed = value.trim();
  if (!NON_NEGATIVE_DECIMAL_REGEX.test(trimmed)) return false;
  return Number.isFinite(Number(trimmed));
}

type UsageCostLog = Pick<
  RequestLog,
  | "inputTokens"
  | "outputTokens"
  | "cacheReadTokens"
  | "cacheCreationTokens"
  | "totalCostUsd"
  | "statusCode"
> &
  Partial<Pick<RequestLog, "costMultiplier">>;

export function hasUsageTokens(log: UsageCostLog): boolean {
  return (
    log.inputTokens > 0 ||
    log.outputTokens > 0 ||
    log.cacheReadTokens > 0 ||
    log.cacheCreationTokens > 0
  );
}

export function isUnpricedUsage(log: UsageCostLog): boolean {
  const totalCost = Number.parseFloat(log.totalCostUsd);
  const multiplier =
    log.costMultiplier == null
      ? undefined
      : Number.parseFloat(log.costMultiplier);
  return (
    log.statusCode >= 200 &&
    log.statusCode < 300 &&
    hasUsageTokens(log) &&
    Number.isFinite(totalCost) &&
    (!Number.isFinite(multiplier) || multiplier !== 0) &&
    totalCost === 0
  );
}
