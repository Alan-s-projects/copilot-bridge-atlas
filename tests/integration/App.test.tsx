import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import App from "@/App";
import { createTestQueryClient } from "../utils/testQueryClient";
const mocks = vi.hoisted(() => ({
  autoSaveSettings: vi.fn().mockResolvedValue(true),
  providers: vi.fn(() => ({
    data: {
      providers: {
        copilot: { id: "copilot", name: "GitHub Copilot", settingsConfig: {} },
      },
      currentProviderId: "copilot",
    },
    isLoading: false,
    refetch: vi.fn(),
  })),
}));
vi.mock("@/lib/query", () => ({
  useProvidersQuery: mocks.providers,
}));
vi.mock("@/lib/api", () => ({
  providersApi: {
    onSwitched: vi.fn().mockResolvedValue(() => {}),
    updateTrayMenu: vi.fn(),
  },
}));
vi.mock("@/hooks/useProxyStatus", () => ({
  useProxyStatus: () => ({ isRunning: false, status: {} }),
}));
vi.mock("@/hooks/useSettings", () => ({
  useSettings: () => ({
    settings: { usageDashboardRefreshIntervalMs: 10000 },
    isLoading: false,
    autoSaveSettings: mocks.autoSaveSettings,
  }),
}));
vi.mock("@/components/providers/CopilotCard", () => ({
  CopilotCard: () => <div>GitHub Copilot</div>,
}));
vi.mock("@/components/overview/BridgeOverview", () => ({
  BridgeOverview: () => <div data-testid="bridge-overview">Overview</div>,
}));
vi.mock("@/components/overview/BridgeWarnings", () => ({
  BridgeWarnings: () => (
    <section data-testid="bridge-warnings">Warnings</section>
  ),
}));
vi.mock("@/components/proxy/ProxyToggle", () => ({
  ProxyToggle: () => (
    <button role="switch" aria-label="Proxy server" aria-checked={false} />
  ),
}));
vi.mock("@/components/settings/SettingsPage", () => ({
  SettingsPage: () => <div data-testid="settings-page">settings</div>,
}));
vi.mock("@/components/usage/UsageDashboard", () => ({
  UsageDashboard: ({
    refreshIntervalMs,
    onRefreshIntervalChange,
  }: {
    refreshIntervalMs: number;
    onRefreshIntervalChange: (next: number) => Promise<boolean>;
  }) => (
    <div data-testid="usage-dashboard">
      Refresh every {refreshIntervalMs}
      <button onClick={() => void onRefreshIntervalChange(5000)}>
        Change refresh interval
      </button>
    </div>
  ),
}));
vi.mock("@/components/providers/CodexSetupSuggestion", () => ({
  CodexSetupSuggestion: () => (
    <div>
      read-only-suggestions
      <input aria-label="Codex TOML file" />
    </div>
  ),
}));
function renderApp() {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <App />
    </QueryClientProvider>,
  );
}
describe("Atlas application scope", () => {
  it("opens the Copilot overview", () => {
    renderApp();
    expect(screen.getByText("GitHub Copilot")).toBeVisible();
    expect(screen.getByTestId("bridge-overview")).toBeVisible();
    const warnings = screen.getByTestId("bridge-warnings");
    const provider = screen.getByText("GitHub Copilot");
    expect(warnings.parentElement).toBe(provider.parentElement);
    expect(
      warnings.compareDocumentPosition(provider) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(mocks.providers).toHaveBeenCalledWith();
    for (const title of [
      "skills.manage",
      "mcp.title",
      "prompts.manage",
      "sessionManager.title",
    ]) {
      expect(screen.queryByTitle(title)).not.toBeInTheDocument();
    }
  });
  it("keeps the navigation and proxy control visible while highlighting only the selected page", async () => {
    renderApp();
    const navigation = screen.getByRole("navigation", {
      name: "Main navigation",
    });
    const proxyControl = screen.getByRole("switch", { name: "Proxy server" });
    const selectedPage = () =>
      within(navigation).getByRole("button", { current: "page" });
    expect(
      within(navigation)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Overview", "Usage", "Connect", "Settings"]);
    expect(selectedPage()).toHaveTextContent("Overview");
    expect(
      within(screen.getByRole("banner")).queryByRole("button", {
        name: "Health check",
      }),
    ).not.toBeInTheDocument();
    fireEvent.click(within(navigation).getByRole("button", { name: "Usage" }));
    expect(await screen.findByTestId("usage-dashboard")).toHaveTextContent(
      "Refresh every 10000",
    );
    expect(selectedPage()).toHaveTextContent("Usage");
    expect(proxyControl).toBeVisible();
    expect(screen.queryByTestId("bridge-overview")).not.toBeInTheDocument();
    expect(screen.queryByTestId("settings-page")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Change refresh interval" }),
    );
    await waitFor(() =>
      expect(mocks.autoSaveSettings).toHaveBeenCalledWith({
        usageDashboardRefreshIntervalMs: 5000,
      }),
    );
    fireEvent.click(
      within(navigation).getByRole("button", { name: "Settings" }),
    );
    expect(screen.getByTestId("settings-page")).toBeVisible();
    expect(selectedPage()).toHaveTextContent("Settings");
    expect(screen.getByRole("main")).toHaveClass("overflow-hidden");
    expect(screen.getByRole("main")).not.toHaveClass("overflow-y-auto");
    expect(proxyControl).toBeVisible();
    expect(screen.queryByTestId("usage-dashboard")).not.toBeInTheDocument();
    fireEvent.click(
      within(navigation).getByRole("button", { name: "Overview" }),
    );
    expect(screen.getByText("GitHub Copilot")).toBeVisible();
    expect(screen.getByTestId("bridge-overview")).toBeVisible();
    expect(selectedPage()).toHaveTextContent("Overview");
    expect(screen.getByRole("main")).toHaveClass("overflow-y-auto");
    expect(proxyControl).toBeVisible();
    expect(screen.queryByTestId("settings-page")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /back/i }),
    ).not.toBeInTheDocument();
  });
  it("opens read-only connection previews through the persistent navigation", async () => {
    renderApp();
    const connect = screen.getByRole("button", { name: "Connect" });
    fireEvent.click(connect);
    expect(await screen.findByText("read-only-suggestions")).toBeVisible();
    expect(connect).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("button", { name: "Overview" })).toBeVisible();
    expect(screen.getByRole("switch", { name: "Proxy server" })).toBeVisible();
    expect(screen.queryByTestId("bridge-overview")).not.toBeInTheDocument();
  });
  it("retains keyboard navigation without leaving the connection page while editing a file path", () => {
    renderApp();
    fireEvent.keyDown(window, { key: ",", ctrlKey: true });
    expect(screen.getByTestId("settings-page")).toBeVisible();
    expect(screen.getByRole("button", { name: "Settings" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    fireEvent.click(screen.getByRole("button", { name: "Connect" }));
    fireEvent.keyDown(
      screen.getByRole("textbox", { name: "Codex TOML file" }),
      {
        key: "Escape",
      },
    );
    expect(screen.getByText("read-only-suggestions")).toBeVisible();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByTestId("bridge-overview")).toBeVisible();
    expect(screen.getByRole("button", { name: "Overview" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });
});
