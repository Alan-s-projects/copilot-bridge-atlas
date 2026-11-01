import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { proxyApi } from "@/lib/api/proxy";
import { proxyEndpoints } from "../fixtures/proxyEndpoints";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
beforeEach(() => vi.mocked(invoke).mockReset());

it("reads the endpoint registry through the parameterless IPC command", async () => {
  vi.mocked(invoke).mockResolvedValue(proxyEndpoints);
  expect(await proxyApi.getProxyEndpoints()).toEqual(proxyEndpoints);
  expect(invoke).toHaveBeenCalledTimes(1);
  expect(invoke).toHaveBeenCalledWith("get_proxy_endpoints");
});

it("preserves an empty registry", async () => {
  vi.mocked(invoke).mockResolvedValue([]);
  expect(await proxyApi.getProxyEndpoints()).toEqual([]);
});

it("propagates IPC errors instead of returning a fabricated inventory", async () => {
  vi.mocked(invoke).mockRejectedValueOnce(new Error("IPC unavailable"));
  await expect(proxyApi.getProxyEndpoints()).rejects.toThrow("IPC unavailable");
});
