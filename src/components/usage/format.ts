import {
  getReadCacheHitRate,
  type InputTokenUsage,
  type RequestLog,
} from "@/types/usage";

export function formatReadCacheHitRate(log: InputTokenUsage): string {
  const rate = getReadCacheHitRate(log);
  return rate == null ? "--" : `${(rate * 100).toFixed(1)}%`;
}

export function formatCostBreakdown(
  log: Pick<
    RequestLog,
    | "inputCostUsd"
    | "outputCostUsd"
    | "cacheReadCostUsd"
    | "cacheCreationCostUsd"
    | "costMultiplier"
  >,
): string {
  return [
    `Fresh input: ${fmtUsd(log.inputCostUsd, 6)}`,
    `Cached Input: ${fmtUsd(log.cacheReadCostUsd, 6)}`,
    `Output: ${fmtUsd(log.outputCostUsd, 6)}`,
    `Cache Write: ${fmtUsd(log.cacheCreationCostUsd, 6)}`,
    `Cost multiplier: x${log.costMultiplier}`,
  ].join("; ");
}

export function parseFiniteNumber(value: unknown): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }

  if (typeof value === "string") {
    const parsed = Number.parseFloat(value);
    return Number.isFinite(parsed) ? parsed : null;
  }

  return null;
}

export function fmtInt(
  value: unknown,
  locale = "en-US",
  fallback: string = "--",
): string {
  const num = parseFiniteNumber(value);
  if (num == null) return fallback;
  return new Intl.NumberFormat(locale).format(Math.trunc(num));
}

export function fmtUsd(
  value: unknown,
  digits: number,
  fallback: string = "--",
): string {
  const num = parseFiniteNumber(value);
  if (num == null) return fallback;
  return `$${num.toFixed(digits)}`;
}

interface OutputTokensPerSecondInput {
  outputTokens: unknown;
  latencyMs: unknown;
  firstTokenMs?: unknown;
  durationMs?: unknown;
}

function getOutputGenerationDurationMs(
  log: OutputTokensPerSecondInput,
): number | null {
  const durationMs = parseFiniteNumber(log.durationMs);
  if (durationMs != null && durationMs > 0) return durationMs;

  const firstTokenMs = parseFiniteNumber(log.firstTokenMs);
  if (firstTokenMs != null) {
    const latencyMs = parseFiniteNumber(log.latencyMs);
    if (latencyMs == null) return null;
    const generationMs = latencyMs - firstTokenMs;
    return generationMs > 0 ? generationMs : null;
  }

  const latencyMs = parseFiniteNumber(log.latencyMs);
  return latencyMs != null && latencyMs > 0 ? latencyMs : null;
}

export function getOutputTokensPerSecond(
  log: OutputTokensPerSecondInput,
): number | null {
  const outputTokens = parseFiniteNumber(log.outputTokens);
  if (outputTokens == null || outputTokens <= 0) return null;

  const durationMs = getOutputGenerationDurationMs(log);
  if (durationMs == null) return null;

  const tps = outputTokens / (durationMs / 1000);
  return Number.isFinite(tps) && tps > 0 ? tps : null;
}

export function formatOutputTokensPerSecond(
  log: OutputTokensPerSecondInput,
): string | null {
  const tps = getOutputTokensPerSecond(log);
  if (tps == null) return null;
  return tps >= 1 ? Math.round(tps).toString() : tps.toFixed(1);
}

/** Compact token counts using English K/M/B units. */
export function formatTokensShort(
  value: number,
  compactDecimals?: 1 | 2,
): string {
  if (!Number.isFinite(value) || value <= 0) return "0";
  const scaled = (unit: number, decimals: number) => {
    const factor = 10 ** decimals;
    return (Math.round(value / (unit / factor)) / factor).toFixed(decimals);
  };
  if (value >= 1e9) return `${scaled(1e9, compactDecimals ?? 2)}B`;
  if (value >= 1e6) return `${scaled(1e6, compactDecimals ?? 2)}M`;
  if (value >= 1e3) return `${scaled(1e3, compactDecimals ?? 1)}K`;
  return value.toLocaleString("en-US");
}
