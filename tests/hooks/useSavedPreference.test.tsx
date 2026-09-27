import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { useSavedPreference } from "@/hooks/useSavedPreference";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

function setup() {
  let stored = "original";
  const gates = [deferred<void>(), deferred<void>()];
  const load = vi.fn(async () => stored);
  const save = vi.fn(async (value: string) => {
    const index = save.mock.calls.length - 1;
    await gates[index].promise;
    stored = value;
    return stored;
  });
  const client = createTestQueryClient();
  const options = {
    queryKey: ["settings", "test-preference"],
    load,
    save,
    optimisticUpdate: (_previous: string | undefined, next: string) => next,
    errorMessage: "Save failed.",
  };
  const hook = renderHook(
    () => ({
      first: useSavedPreference(options),
      second: useSavedPreference(options),
    }),
    {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
  return { ...hook, gates, load, save, readStored: () => stored };
}

describe("Ordered preference writes", () => {
  it("serializes writes across consumers without replacing a newer optimistic selection", async () => {
    const { result, gates, save, readStored } = setup();
    await waitFor(() =>
      expect(result.current.first.query.data).toBe("original"),
    );
    act(() => {
      result.current.first.mutation.mutate("older");
      result.current.second.mutation.mutate("latest");
    });
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(result.current.first.query.data).toBe("latest"));
    expect(result.current.second.saving).toBe(true);
    await act(async () => gates[0].resolve());
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    expect(result.current.first.query.data).toBe("latest");
    await act(async () => gates[1].resolve());
    await waitFor(() => expect(result.current.first.saving).toBe(false));
    expect(readStored()).toBe("latest");
    expect(result.current.second.query.data).toBe("latest");
  });

  it.each([true, false])(
    "restores committed data after the last write fails (first succeeded=%s)",
    async (firstSucceeds) => {
      const { result, gates, save } = setup();
      await waitFor(() =>
        expect(result.current.first.query.data).toBe("original"),
      );
      act(() => {
        result.current.first.mutation.mutate("older");
        result.current.second.mutation.mutate("latest");
      });
      await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
      await act(async () => {
        if (firstSucceeds) gates[0].resolve();
        else gates[0].reject(new Error("First save failed"));
      });
      await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
      await act(async () => gates[1].reject(new Error("Last save failed")));
      await waitFor(() => expect(result.current.first.saving).toBe(false));
      expect(result.current.second.query.data).toBe(
        firstSucceeds ? "older" : "original",
      );
    },
  );

  it("continues with a later successful write after an earlier failure", async () => {
    const { result, gates, save, readStored } = setup();
    await waitFor(() =>
      expect(result.current.first.query.data).toBe("original"),
    );
    act(() => {
      result.current.first.mutation.mutate("older");
      result.current.second.mutation.mutate("latest");
    });
    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    await act(async () => gates[0].reject(new Error("First save failed")));
    await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
    await act(async () => gates[1].resolve());
    await waitFor(() => expect(result.current.first.saving).toBe(false));
    expect(readStored()).toBe("latest");
    expect(result.current.second.query.data).toBe("latest");
  });
});
