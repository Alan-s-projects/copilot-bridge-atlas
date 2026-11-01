import {
  fireEvent,
  render as rtlRender,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactElement } from "react";
import { proxyEndpoints } from "../fixtures/proxyEndpoints";

function render(ui: ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return rtlRender(
    <QueryClientProvider client={client}>{ui}</QueryClientProvider>,
  );
}
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ProxyPanel } from "@/components/proxy/ProxyPanel";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  endpoints: vi.fn(),
  running: false,
  copy: vi.fn(),
  config: {
    claude: {
      address: "127.0.0.1",
      port: 15723,
      sonnet: "",
      opus: "",
      haiku: "",
      fable: "",
    },
    proxyEnabled: true,
    listenAddress: "127.0.0.1",
    listenPort: 15722,
  },
}));

vi.mock("@/lib/api/proxy", () => ({
  proxyApi: { getProxyEndpoints: mocks.endpoints },
}));

beforeEach(() => {
  mocks.endpoints.mockReset().mockResolvedValue(proxyEndpoints);
  mocks.running = false;
  mocks.config.listenAddress = "127.0.0.1";
  mocks.config.claude.address = "127.0.0.1";
});

vi.mock("@/lib/query", () => ({
  useProvidersQuery: () => ({ data: { providers: {}, currentProviderId: "" } }),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@/lib/query/proxy", () => ({
  useGlobalProxyConfig: () => ({ data: mocks.config }),
  useProxyStatusQuery: () => ({
    data: {
      running: mocks.running,
      total_requests: 0,
      active_connections: 0,
      success_rate: 100,
    },
  }),
  useUpdateGlobalProxyConfig: () => ({
    mutateAsync: (update: any) =>
      mocks.save(typeof update === "function" ? update(mocks.config) : update),
    mutate: mocks.save,
    isPending: false,
  }),
}));
vi.mock("@/lib/clipboard", () => ({ copyText: mocks.copy }));

describe("ProxyPanel auto-save", () => {
  beforeEach(() => {
    mocks.save.mockReset().mockResolvedValue(undefined);
  });

  it("auto-saves listener edits and removes the Save button", async () => {
    render(<ProxyPanel />);
    const address = screen.getByRole("textbox", {
      name: "Responses API listen address",
    });
    await waitFor(() => expect(address).toHaveValue("127.0.0.1"));

    fireEvent.change(address, { target: { value: "0.0.0.0" } });

    expect(
      screen.queryByRole("button", { name: "Save" }),
    ).not.toBeInTheDocument();
    await waitFor(
      () =>
        expect(mocks.save).toHaveBeenCalledWith({
          ...mocks.config,
          listenAddress: "0.0.0.0",
          listenPort: 15722,
        }),
      { timeout: 1500 },
    );
  });

  it("does not offer a recording toggle or duplicate overview counters", () => {
    render(<ProxyPanel />);
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    for (const label of [
      "Record requests for the usage dashboard",
      "Requests",
      "Active connections",
      "Success rate",
    ]) {
      expect(screen.queryByText(label)).not.toBeInTheDocument();
    }
    expect(mocks.save).not.toHaveBeenCalled();
  });

  it("flushes a pending listener edit when the settings panel closes", async () => {
    const { unmount } = render(<ProxyPanel />);
    const address = screen.getByRole("textbox", {
      name: "Responses API listen address",
    });
    fireEvent.change(address, { target: { value: "0.0.0.0" } });

    unmount();

    await waitFor(() =>
      expect(mocks.save).toHaveBeenCalledWith({
        ...mocks.config,
        listenAddress: "0.0.0.0",
        listenPort: 15722,
      }),
    );
  });
});

it("shows two copyable URLs without mappings without per-server switches or display-name inputs", () => {
  render(<ProxyPanel />);
  expect(screen.getByRole("region", { name: "Responses API" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Messages API" })).toBeVisible();
  expect(screen.getByText("http://127.0.0.1:15723")).toBeVisible();
  expect(screen.queryAllByRole("combobox")).toHaveLength(0);
  expect(screen.queryByText("Display name")).not.toBeInTheDocument();
  expect(screen.queryByRole("switch")).not.toBeInTheDocument();
});

it("loads all twelve registry entries with both directories and every row collapsed", async () => {
  render(<ProxyPanel />);
  const responses = within(
    screen.getByRole("region", { name: "Responses API" }),
  );
  const messages = within(screen.getByRole("region", { name: "Messages API" }));
  const showResponses = await responses.findByRole("button", {
    name: "Show 9 endpoints",
  });
  const showMessages = await messages.findByRole("button", {
    name: "Show 3 endpoints",
  });
  expect(showResponses).toHaveAttribute("aria-expanded", "false");
  expect(showMessages).toHaveAttribute("aria-expanded", "false");
  expect(
    screen.queryByRole("button", { name: / details$/ }),
  ).not.toBeInTheDocument();
  fireEvent.click(showResponses);
  expect(responses.getAllByRole("button", { name: / details$/ })).toHaveLength(
    9,
  );
  expect(
    messages.queryByRole("button", { name: / details$/ }),
  ).not.toBeInTheDocument();
  fireEvent.click(showMessages);
  expect(messages.getAllByRole("button", { name: / details$/ })).toHaveLength(
    3,
  );
  for (const row of screen.getAllByRole("button", { name: / details$/ })) {
    expect(row).toHaveAttribute("aria-expanded", "false");
  }
  const first = responses.getByRole("button", {
    name: "POST /v1/responses details",
  });
  const second = responses.getByRole("button", {
    name: "WS /v1/responses details",
  });
  fireEvent.click(first);
  fireEvent.click(second);
  expect(screen.getByText("Description 0")).toBeVisible();
  expect(screen.getByText("Description 3")).toBeVisible();
  expect(within(second).getByText("Not implemented")).toBeVisible();
  expect(within(first).getByText("Available")).toBeVisible();
  fireEvent.click(first);
  expect(screen.getByText("Description 0")).not.toBeVisible();
  expect(screen.getByText("Description 3")).toBeVisible();
  expect(screen.getByText("Sample request 3")).toBeVisible();
  expect(screen.getByText("Sample response 3")).toBeVisible();
  expect(screen.getAllByRole("button", { name: /Copy/ })).toHaveLength(2);
  for (const removed of [
    /refresh/i,
    /^search$/i,
    /filter/i,
    /pricing/i,
    /alias/i,
  ]) {
    expect(
      screen.queryByRole("button", { name: removed }),
    ).not.toBeInTheDocument();
  }
  expect(screen.queryByRole("searchbox")).not.toBeInTheDocument();
  expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  expect(mocks.endpoints).toHaveBeenCalledTimes(1);
});

it("shows load failures without inventing endpoints", async () => {
  mocks.endpoints.mockRejectedValue(new Error("IPC unavailable"));
  render(<ProxyPanel />);
  expect(await screen.findAllByRole("alert")).toHaveLength(2);
  expect(screen.getAllByText("Could not load endpoints.")).toHaveLength(2);
  expect(
    screen.queryByRole("button", { name: /Show .* endpoints/ }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: / details$/ }),
  ).not.toBeInTheDocument();
});

it("shows an empty directory instead of a production fallback", async () => {
  mocks.endpoints.mockResolvedValue([]);
  render(<ProxyPanel />);
  const toggles = await screen.findAllByRole("button", {
    name: "Show 0 endpoints",
  });
  fireEvent.click(toggles[0]);
  expect(screen.getAllByText("No endpoints available.")[0]).toBeVisible();
  expect(
    screen.queryByRole("button", { name: / details$/ }),
  ).not.toBeInTheDocument();
});

it("preserves running read-only fields and IPv6 base URL copies", async () => {
  mocks.running = true;
  mocks.config.listenAddress = "::1";
  mocks.config.claude.address = "[::1]";
  render(<ProxyPanel />);
  await screen.findByRole("button", { name: "Show 9 endpoints" });
  expect(screen.getAllByText("Running")).toHaveLength(2);
  expect(
    screen.getByRole("textbox", { name: "Responses API listen address" }),
  ).toBeDisabled();
  expect(
    screen.getByRole("textbox", { name: "Messages API port" }),
  ).toBeDisabled();
  fireEvent.click(
    screen.getByRole("button", { name: "Copy Responses API URL" }),
  );
  expect(mocks.copy).toHaveBeenLastCalledWith("http://[::1]:15722/v1");
  fireEvent.click(
    screen.getByRole("button", { name: "Copy Messages API URL" }),
  );
  expect(mocks.copy).toHaveBeenLastCalledWith("http://[::1]:15723");
});
