import type { ReactNode } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import {
  focusManager,
  QueryClient,
  QueryClientProvider,
  useQuery,
} from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { useUsageEventBridge } from "@/hooks/useUsageEventBridge";

const mocks = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@/lib/windowActivity", () => ({ useWindowActive: () => true }));

afterEach(() => focusManager.setFocused(undefined));

it("coalesces events during existing fetches without cancelling results or losing the trailing update", async () => {
  focusManager.setFocused(true);
  let onUsage!: () => void;
  const off = vi.fn();
  mocks.listen.mockImplementation((_event, callback) => {
    onUsage = callback;
    return Promise.resolve(off);
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: Infinity } },
  });
  const queryKey = ["usage", "test"];
  client.setQueryData(queryKey, 0);
  const complete: Array<(value: number) => void> = [];
  const queryFn = vi.fn(
    () => new Promise<number>((resolve) => complete.push(resolve)),
  );
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { result, unmount } = renderHook(
    () => {
      useUsageEventBridge();
      return useQuery({ queryKey, queryFn });
    },
    { wrapper },
  );

  await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(1));
  act(() => {
    for (let i = 0; i < 10; i++) onUsage();
  });
  expect(queryFn).toHaveBeenCalledTimes(1);
  await act(async () => complete[0](1));
  await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(result.current.data).toBe(1));

  act(() => {
    for (let i = 0; i < 10; i++) onUsage();
  });
  expect(queryFn).toHaveBeenCalledTimes(2);
  await act(async () => complete[1](2));
  await waitFor(() => expect(queryFn).toHaveBeenCalledTimes(3));
  await act(async () => complete[2](3));
  await waitFor(() => expect(result.current.data).toBe(3));
  expect(queryFn).toHaveBeenCalledTimes(3);

  unmount();
  expect(off).toHaveBeenCalledOnce();
  onUsage();
  expect(queryFn).toHaveBeenCalledTimes(3);
  client.clear();
});
