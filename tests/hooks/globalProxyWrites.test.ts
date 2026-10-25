import { expect, it, vi } from "vitest";
import { proxyApi } from "@/lib/api/proxy";
import { defaultClaude } from "@/hooks/useClaudeMappings";
import type { GlobalProxyConfig } from "@/types/proxy";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
it("merges queued mapping and listener changes against the latest saved config", async () => {
  let current: GlobalProxyConfig = {
    listenAddress: "127.0.0.1",
    listenPort: 15722,
    proxyEnabled: true,
    claude: { ...defaultClaude },
  };
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let writes = 0;
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "get_global_proxy_config") return structuredClone(current);
    if (++writes === 1) await pending;
    current = args.config;
  });
  const first = proxyApi.updateGlobalProxyConfig((c) => ({
    ...c,
    claude: { ...c.claude!, sonnet: "gpt-6-astra" },
  }));
  const second = proxyApi.updateGlobalProxyConfig((c) => ({
    ...c,
    listenPort: 18000,
    claude: { ...c.claude!, port: 18001 },
  }));
  const third = proxyApi.updateGlobalProxyConfig((c) => ({
    ...c,
    claude: { ...c.claude!, opus: "gpt-6-luna" },
  }));
  release();
  await Promise.all([first, second, third]);
  expect(current).toMatchObject({
    listenPort: 18000,
    claude: { port: 18001, sonnet: "gpt-6-astra", opus: "gpt-6-luna" },
  });
  mocks.invoke.mockRejectedValueOnce(new Error("read failed"));
  await expect(proxyApi.updateGlobalProxyConfig((c) => c)).rejects.toThrow(
    "read failed",
  );
  await expect(proxyApi.updateGlobalProxyConfig((c) => c)).resolves.toEqual(
    current,
  );
});
