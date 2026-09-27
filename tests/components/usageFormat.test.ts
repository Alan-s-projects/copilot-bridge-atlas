import { describe, expect, it } from "vitest";
import {
  formatOutputTokensPerSecond,
  formatTokensShort,
  getOutputTokensPerSecond,
  formatReadCacheHitRate,
  formatCostBreakdown,
} from "@/components/usage/format";
import { getFreshInputTokens } from "@/types/usage";

describe("usage format helpers", () => {
  it("uses server-normalized fresh input and excludes output from read hit rate", () => {
    const log = {
      appType: "codex",
      inputTokens: 1000,
      freshInputTokens: 100,
      cacheReadTokens: 800,
      cacheCreationTokens: 100,
      outputTokens: 9000,
    };
    expect(getFreshInputTokens(log)).toBe(100);
    expect(formatReadCacheHitRate(log)).toBe("80.0%");
    const withoutOutput = { ...log, outputTokens: 0 };
    expect(formatReadCacheHitRate(withoutOutput)).toBe("80.0%");
    expect(
      formatReadCacheHitRate({
        ...log,
        inputTokens: 0,
        freshInputTokens: 0,
        cacheReadTokens: 0,
        cacheCreationTokens: 0,
      }),
    ).toBe("--");
  });

  it("preserves explicit zero and legacy fresh-input fallbacks", () => {
    expect(
      getFreshInputTokens({
        appType: "codex",
        inputTokens: 1000,
        freshInputTokens: 0,
        cacheReadTokens: 800,
      }),
    ).toBe(0);
    expect(
      getFreshInputTokens({
        appType: "codex",
        inputTokens: 1000,
        cacheReadTokens: 800,
      }),
    ).toBe(200);
    expect(
      getFreshInputTokens({
        appType: "codex",
        inputTokens: 100,
        cacheReadTokens: 800,
      }),
    ).toBe(100);
  });

  it("shows all stored cost components and the multiplier without repricing", () => {
    expect(
      formatCostBreakdown({
        inputCostUsd: "0.003",
        outputCostUsd: "0.0075",
        cacheReadCostUsd: "0.00006",
        cacheCreationCostUsd: "0.000375",
        costMultiplier: "1.5",
      }),
    ).toBe(
      "Fresh input: $0.003000; Cached Input: $0.000060; Output: $0.007500; Cache Write: $0.000375; Cost multiplier: x1.5",
    );
  });

  it("formats compact English token units", () => {
    expect(formatTokensShort(12_345)).toBe("12.3K");
    expect(formatTokensShort(123_456_789, 2)).toBe("123.46M");
  });

  it("calculates streaming TPS from generation duration after first token", () => {
    expect(
      getOutputTokensPerSecond({
        outputTokens: 120,
        latencyMs: 10_000,
        firstTokenMs: 4_000,
      }),
    ).toBe(20);
  });

  it("prefers explicit durationMs for output TPS", () => {
    expect(
      getOutputTokensPerSecond({
        outputTokens: 120,
        latencyMs: 10_000,
        firstTokenMs: 4_000,
        durationMs: 3_000,
      }),
    ).toBe(40);
  });

  it("falls back to full latency when first token timing is missing", () => {
    expect(
      getOutputTokensPerSecond({
        outputTokens: 120,
        latencyMs: 10_000,
      }),
    ).toBe(12);
  });

  it("does not show TPS without positive tokens or duration", () => {
    expect(
      formatOutputTokensPerSecond({
        outputTokens: 0,
        latencyMs: 10_000,
      }),
    ).toBeNull();
    expect(
      formatOutputTokensPerSecond({
        outputTokens: 120,
        latencyMs: 4_000,
        firstTokenMs: 4_000,
      }),
    ).toBeNull();
  });

  it("formats TPS with integer or single-decimal precision", () => {
    expect(
      formatOutputTokensPerSecond({
        outputTokens: 121,
        latencyMs: 10_000,
      }),
    ).toBe("12");
    expect(
      formatOutputTokensPerSecond({
        outputTokens: 1,
        latencyMs: 4_000,
      }),
    ).toBe("0.3");
  });
});
