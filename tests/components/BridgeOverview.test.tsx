import { QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { BridgeOverview } from "@/components/overview/BridgeOverview";
import { BridgeWarnings } from "@/components/overview/BridgeWarnings";
import { OverviewRefreshButton } from "@/components/overview/OverviewRefreshButton";
import type { ProxyStatus } from "@/types/proxy";
import { createTestQueryClient } from "../utils/testQueryClient";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  summary: vi.fn(),
  logs: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("@/lib/windowActivity", () => ({ useWindowActive: () => true }));
vi.mock("@/lib/api/usage", () => ({
  usageApi: { getUsageSummary: mocks.summary, getRequestLogs: mocks.logs },
}));
vi.mock("@/lib/query/proxy", () => ({
  proxyKeys: { status: ["proxyStatus"], globalConfig: ["globalProxyConfig"] },
  useGlobalProxyConfig: () => ({
    data: { listenAddress: "0.0.0.0", listenPort: 15722 },
  }),
}));

const status: ProxyStatus = {
  running: true,
  address: "0.0.0.0",
  port: 15722,
  active_connections: 2,
  total_requests: 12,
  success_requests: 11,
  failed_requests: 1,
  success_rate: 91.7,
  uptime_seconds: 600,
  current_provider: "GitHub Copilot",
  current_provider_id: "copilot",
  last_request_at: null,
  last_error: null,
};
const snapshot = {
  summary: {
    totalRequests: 12,
    totalCost: "1.25",
    totalInputTokens: 200,
    totalOutputTokens: 50,
    totalCacheCreationTokens: 0,
    totalCacheReadTokens: 800,
    successRate: 91.7,
    realTotalTokens: 1050,
    cacheHitRate: 0.8,
  },
  recent: {
    data: [
      {
        requestId: "ok",
        createdAt: 1790397000,
        model: "gpt-6-astra",
        requestedReasoningEffort: "ultra",
        pricingTier: "long_context",
        appliedReasoningEffort: "max",
        statusCode: 200,
        latencyMs: 2500,
        firstTokenMs: 500,
        inputTokens: 1000,
        freshInputTokens: 300,
        cacheReadTokens: 600,
        cacheCreationTokens: 100,
        outputTokens: 200,
        totalCostUsd: "0.01",
      },
      {
        requestId: "failed",
        createdAt: 1790396990,
        model: "gpt-6-luna",
        statusCode: 400,
        latencyMs: 180,
        inputTokens: 0,
        outputTokens: 0,
        totalCostUsd: "0",
      },
    ],
  },
};
function renderOverview(proxyStatus = status) {
  const client = createTestQueryClient();
  const rendered = render(
    <QueryClientProvider client={client}>
      <OverviewRefreshButton />
      <BridgeWarnings status={proxyStatus} />
      <BridgeOverview status={proxyStatus} />
    </QueryClientProvider>,
  );
  return { ...rendered, client };
}
beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue({
    configured: true,
    configExists: true,
    configPath: "C:/Users/test/.codex/config.toml",
  });
  mocks.summary.mockReset().mockResolvedValue(snapshot.summary);
  mocks.logs.mockReset().mockResolvedValue(snapshot.recent);
});

describe("read-only bridge overview", () => {
  it("shows the local address, token-based cache reuse, cost, activity and recent statuses", async () => {
    renderOverview();
    const proxy = within(screen.getByRole("region", { name: "Proxy" }));
    const usage = within(screen.getByRole("region", { name: "Today's usage" }));
    const requests = within(screen.getByRole("region", { name: "Requests" }));
    expect(
      screen.queryByRole("heading", { name: "Overview" }),
    ).not.toBeInTheDocument();
    expect(proxy.getByText("http://127.0.0.1:15722/v1")).toBeVisible();
    expect(proxy.getByText("Proxy running")).toBeVisible();
    expect(await usage.findByText("$1.3")).toBeVisible();
    expect(usage.getByText("80.0%")).toBeVisible();
    expect(usage.getByText("91.7%")).toBeVisible();
    expect(proxy.getByText("Active requests:").textContent).toContain("2");
    const table = requests.getByRole("table", {
      name: "Latest 5 completed requests",
    });
    expect(table).toHaveClass("[&_td]:py-2", "[&_th]:h-10");
    expect(within(table).getByText("gpt-6-astra")).toBeVisible();
    const headings = within(table).getAllByRole("columnheader");
    expect(headings[1]).toHaveTextContent("Billing Model");
    expect(headings[5]).toHaveTextContent("Pricing Tier");
    expect(headings[2]).toHaveTextContent("Reasoning(requested/applied)");
    const rows = within(table).getAllByRole("row");
    expect(within(rows[1]).getAllByRole("cell")[5]).toHaveTextContent(
      "Long context",
    );
    expect(within(rows[1]).getAllByRole("cell")[2]).toHaveTextContent(
      "ultra / max",
    );
    expect(within(rows[2]).getAllByRole("cell")[5]).toHaveTextContent("--");
    expect(within(rows[2]).getAllByRole("cell")[2]).toHaveTextContent(
      "\u2014 / \u2014",
    );
    expect(within(table).getByText("400")).toBeVisible();
    const firstCells = within(rows[1]).getAllByRole("cell");
    expect(firstCells).toHaveLength(7);
    expect(firstCells[3]).toHaveTextContent("200");
    expect(firstCells[6]).toHaveTextContent("$0.0100");
    expect(within(table).getByText("2.5s")).toBeVisible();
    expect(firstCells[4]).toHaveTextContent(/^2\.5s$/);
    expect(headings.map((heading) => heading.textContent)).toEqual([
      "Time",
      "Billing Model",
      "Reasoning(requested/applied)",
      "Status",
      "Duration",
      "Pricing Tier",
      "Cost",
    ]);
    expect(mocks.logs).toHaveBeenCalledWith({ appType: "codex" }, 0, 5);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    expect(
      proxy.queryByRole("button", { name: "Refresh overview" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_codex_setup_suggestion", {
        configPath: null,
      }),
    );
    expect(
      mocks.invoke.mock.calls.every(
        ([command]) => command === "get_codex_setup_suggestion",
      ),
    ).toBe(true);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Token costs are estimates, not your Copilot bill/),
    ).not.toBeInTheDocument();
  });

  it("rounds Overview cost estimates to one decimal place", async () => {
    mocks.summary.mockResolvedValue({
      ...snapshot.summary,
      totalCost: "553.0626",
    });
    renderOverview();
    expect(await screen.findByText("$553.1")).toBeVisible();
  });

  it("shows at most five requests even when an older cached response has more", async () => {
    mocks.logs.mockResolvedValue({
      data: Array.from({ length: 8 }, (_, index) => ({
        ...snapshot.recent.data[0],
        requestId: `request-${index}`,
        model: `model-${index}`,
      })),
    });
    renderOverview();
    const table = await screen.findByRole("table", {
      name: "Latest 5 completed requests",
    });
    await within(table).findByText("model-0");
    expect(within(table).getAllByRole("row")).toHaveLength(6);
    expect(within(table).getByText("model-4")).toBeVisible();
    expect(within(table).queryByText("model-5")).not.toBeInTheDocument();
  });

  it.each([true, false])(
    "warns about a stopped proxy and mismatched/missing detected TOML (exists=%s)",
    async (configExists) => {
      mocks.invoke.mockResolvedValue({
        configured: false,
        configExists,
        configPath: "C:/Users/test/.codex/config.toml",
      });
      renderOverview({ ...status, running: false, active_connections: 0 });
      const proxy = within(screen.getByRole("region", { name: "Proxy" }));
      const warnings = within(
        screen.getByRole("region", { name: "Connection warnings" }),
      );
      expect(warnings.getByText("Proxy is stopped")).toBeVisible();
      expect(
        warnings.getByText(/Turn on the proxy switch in the top bar/),
      ).toBeVisible();
      expect(
        await warnings.findByText("Codex is not connected to Atlas"),
      ).toBeVisible();
      expect(
        screen.getByText("C:/Users/test/.codex/config.toml"),
      ).toBeVisible();
      expect(
        screen.getByText(
          configExists ? /detected TOML points elsewhere/ : /No TOML was found/,
        ),
      ).toBeVisible();
      expect(screen.getAllByRole("alert")).toHaveLength(2);
      expect(proxy.queryByRole("alert")).not.toBeInTheDocument();
    },
  );

  it("reports read failures separately from a known misconfiguration and retains the last usage snapshot", async () => {
    mocks.invoke.mockRejectedValue(new Error("Unable to read file"));
    const { client } = renderOverview();
    expect(await screen.findByText("$1.3")).toBeVisible();
    mocks.summary.mockRejectedValue(new Error("Database unavailable"));
    await act(() =>
      client.invalidateQueries({ queryKey: ["bridge-overview"] }),
    );
    expect(
      await screen.findByText("Could not check Codex configuration"),
    ).toBeVisible();
    expect(
      screen.queryByText("Codex is not connected to Atlas"),
    ).not.toBeInTheDocument();
    expect(await screen.findByText(/Showing the last snapshot/)).toBeVisible();
    expect(screen.getByText("$1.3")).toBeVisible();
  });

  it("does not invent success or cache rates when no requests are recorded", async () => {
    mocks.summary.mockResolvedValue({
      ...snapshot.summary,
      totalRequests: 0,
      totalInputTokens: 0,
      totalCacheReadTokens: 0,
    });
    mocks.logs.mockResolvedValue({ data: [] });
    renderOverview();
    expect(await screen.findByText("No data")).toBeVisible();
    expect(screen.getByRole("cell")).toHaveAttribute("colspan", "7");
    for (const label of ["Success today", "Cache reuse today"]) {
      expect(screen.getByText(label).nextElementSibling).toHaveTextContent("—");
    }
  });

  it("explicitly refreshes usage, recent requests and the read-only connection check", async () => {
    renderOverview();
    expect(await screen.findByText("$1.3")).toBeVisible();
    const refresh = screen.getByRole("button", { name: "Refresh overview" });
    await waitFor(() => expect(refresh).toBeEnabled());
    mocks.summary.mockResolvedValue({ ...snapshot.summary, totalCost: "2.5" });
    fireEvent.click(refresh);
    expect(await screen.findByText("$2.5")).toBeVisible();
    expect(mocks.summary).toHaveBeenCalledTimes(2);
    expect(mocks.logs).toHaveBeenCalledTimes(2);
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });
});
