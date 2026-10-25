import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  useUsageSummary,
  useUsageTrends,
  useModelStats,
  useUnpricedModelUsage,
  useRequestLogs,
  useUsageClients,
  usageKeys,
} from "@/lib/query/usage";

const api = vi.hoisted(() => ({
  getUsageSummary: vi.fn(async (...args: unknown[]) => ({
    totalRequests: args[5] === "Codex" ? 1 : 2,
  })),
  getUsageTrends: vi.fn(async (..._args: unknown[]) => ({ buckets: [] })),
  getModelStats: vi.fn(async (..._args: unknown[]) => []),
  getUnpricedModelUsage: vi.fn(async (..._args: unknown[]) => []),
  getRequestLogs: vi.fn(async (..._args: unknown[]) => ({
    data: [],
    total: 0,
  })),
  getUsageClients: vi.fn(async () => ["Codex", "Claude Desktop"]),
}));
vi.mock("@/lib/api/usage", () => ({ usageApi: api }));

describe("Usage client query isolation", () => {
  it("passes the client to every API and separates cached summaries", async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { result, rerender } = renderHook(
      ({ clientName }) => {
        const range = { preset: "this-week" as const };
        const filters = { appType: "codex", model: "gpt-6-astra", clientName };
        const options = { refetchInterval: false as const };
        return {
          summary: useUsageSummary(range, filters, options),
          trends: useUsageTrends(range, filters, options),
          models: useModelStats(range, filters, options),
          unpriced: useUnpricedModelUsage(range, filters, options),
          logs: useRequestLogs({ range, filters, options }),
          clients: useUsageClients(),
        };
      },
      { initialProps: { clientName: "Codex" }, wrapper },
    );
    await waitFor(() =>
      expect(
        Object.values(result.current).every((query) => query.isSuccess),
      ).toBe(true),
    );
    expect(result.current.summary.data?.totalRequests).toBe(1);
    expect(api.getUsageSummary.mock.lastCall?.[5]).toBe("Codex");
    expect(api.getUsageTrends.mock.lastCall?.[6]).toBe("Codex");
    expect(api.getModelStats.mock.lastCall?.[5]).toBe("Codex");
    expect(api.getUnpricedModelUsage.mock.lastCall?.[5]).toBe("Codex");
    expect(api.getRequestLogs.mock.lastCall?.[0]).toMatchObject({
      clientName: "Codex",
      model: "gpt-6-astra",
    });
    rerender({ clientName: "Claude Desktop" });
    await waitFor(() =>
      expect(result.current.summary.data?.totalRequests).toBe(2),
    );
    expect(api.getUsageSummary.mock.lastCall?.[5]).toBe("Claude Desktop");
  });

  it("includes client identity in every cache key without colliding with model", () => {
    for (const factory of [
      usageKeys.summary,
      usageKeys.modelStats,
      usageKeys.unpricedModelUsage,
    ]) {
      expect(
        factory("today", undefined, undefined, {
          clientName: "Codex",
          model: "x",
        }),
      ).not.toEqual(
        factory("today", undefined, undefined, {
          clientName: "Claude Code",
          model: "x",
        }),
      );
    }
    expect(
      usageKeys.trends("today", undefined, undefined, { clientName: "Codex" }),
    ).not.toEqual(
      usageKeys.trends("today", undefined, undefined, {
        clientName: "Claude Code",
      }),
    );
    expect(
      usageKeys.logs({ preset: "today", clientName: "Codex" }, 0, 20),
    ).not.toEqual(
      usageKeys.logs({ preset: "today", clientName: "Claude Code" }, 0, 20),
    );
  });
});
