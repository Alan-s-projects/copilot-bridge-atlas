import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { ClaudeModelMapping } from "@/components/settings/ClaudeModelMapping";
import { useClaudeMappings, defaultClaude } from "@/hooks/useClaudeMappings";
import { MappingCountBadge } from "@/components/settings/MappingCountBadge";
import type { GlobalProxyConfig } from "@/types/proxy";

const mocks = vi.hoisted(() => ({
  config: undefined as GlobalProxyConfig | undefined,
  save: vi.fn(),
  error: false,
  models: [] as { model: string; enabled?: boolean; available?: boolean }[],
}));
vi.mock("@/lib/query", () => ({
  useProvidersQuery: () => ({
    data: {
      providers: {
        p: { settingsConfig: { modelCatalog: { models: mocks.models } } },
      },
      currentProviderId: "p",
    },
    refetch: vi.fn(),
    isError: false,
  }),
}));
vi.mock("@/lib/query/proxy", () => ({
  useGlobalProxyConfig: () => ({
    data: mocks.config,
    isError: mocks.error,
    refetch: vi.fn(),
  }),
  useUpdateGlobalProxyConfig: () => ({ mutateAsync: mocks.save }),
}));
function Page() {
  const { count } = useClaudeMappings();
  return (
    <>
      <MappingCountBadge count={count} />
      <ClaudeModelMapping />
    </>
  );
}
beforeEach(() => {
  mocks.config = {
    listenAddress: "127.0.0.1",
    listenPort: 15722,
    proxyEnabled: true,
    claude: { ...defaultClaude },
  };
  mocks.models = [{ model: "gpt-6-astra" }, { model: "gpt-6-luna" }];
  mocks.error = false;
  mocks.save.mockReset().mockImplementation(async (update) => {
    mocks.config = update(mocks.config);
  });
});
it("shows a single Saved status in the section header", () => {
  Object.assign(mocks.config!.claude!, {
    sonnet: "gpt-6-astra",
    opus: "gpt-6-astra",
    haiku: "gpt-6-astra",
    fable: "gpt-6-astra",
  });
  render(<Page />);
  expect(screen.getAllByText("Saved")).toHaveLength(1);
  const heading = screen.getByRole("heading", { name: "Claude model mapping" });
  expect(heading.parentElement).toContainElement(screen.getByRole("status"));
  expect(
    screen.queryByText(/Choose an enabled Copilot model/),
  ).not.toBeInTheDocument();
});

it("keeps the shared status saving until concurrent field writes finish", async () => {
  const finishes: (() => void)[] = [];
  mocks.save.mockImplementation(async (update) => {
    await new Promise<void>((resolve) => finishes.push(resolve));
    mocks.config = update(mocks.config);
  });
  render(<Page />);
  fireEvent.change(screen.getByLabelText("sonnet"), {
    target: { value: "gpt-6-astra" },
  });
  fireEvent.change(screen.getByLabelText("opus"), {
    target: { value: "gpt-6-luna" },
  });
  expect(screen.getAllByText("Saving…")).toHaveLength(1);
  await act(async () => finishes[0]());
  expect(screen.getByRole("status")).toHaveTextContent("Saving…");
  expect(screen.queryByText("Saved")).not.toBeInTheDocument();
  await act(async () => finishes[1]());
  expect(screen.getByRole("status")).toHaveTextContent("Saved");
});

it("counts each unset role, preserves explicit choices and clears only after saving", async () => {
  const { rerender } = render(<Page />);
  expect(
    screen.getByLabelText("4 Claude mappings need attention"),
  ).toHaveTextContent("4");
  expect(
    screen.queryByText(/Automatic|Use Sonnet mapping/),
  ).not.toBeInTheDocument();
  for (const role of ["sonnet", "opus", "haiku", "fable"]) {
    fireEvent.change(screen.getByLabelText(role, { exact: false }), {
      target: { value: "gpt-6-astra" },
    });
    await waitFor(() =>
      expect(screen.queryByText("Saving…")).not.toBeInTheDocument(),
    );
    rerender(<Page />);
  }
  expect(
    screen.queryByLabelText(/Claude mappings? need/),
  ).not.toBeInTheDocument();
  expect(mocks.save).toHaveBeenCalledTimes(4);
  expect(mocks.config?.claude).toMatchObject({
    address: "127.0.0.1",
    port: 15723,
    sonnet: "gpt-6-astra",
    opus: "gpt-6-astra",
    haiku: "gpt-6-astra",
    fable: "gpt-6-astra",
  });
  mocks.models[0].enabled = false;
  rerender(<Page />);
  expect(
    screen.getByLabelText("4 Claude mappings need attention"),
  ).toBeVisible();
  expect(
    screen.getAllByText("Enable this model below or choose another model."),
  ).toHaveLength(4);
  expect(screen.getByLabelText("sonnet")).toHaveValue("gpt-6-astra");
});
it("retains a failed selection for retry and does not claim it is saved", async () => {
  mocks.save.mockRejectedValueOnce(new Error("disk full"));
  const { rerender } = render(<Page />);
  fireEvent.change(screen.getByLabelText("sonnet"), {
    target: { value: "gpt-6-astra" },
  });
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
  expect(
    screen.getByLabelText("4 Claude mappings need attention"),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(mocks.save).toHaveBeenCalledTimes(2));
  await waitFor(() =>
    expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
  );
  rerender(<Page />);
  expect(
    screen.getByLabelText("3 Claude mappings need attention"),
  ).toBeVisible();
});
it("does not show a missing count for a failed or unloaded configuration", () => {
  mocks.config = undefined;
  const { rerender } = render(<Page />);
  expect(screen.getByText("Loading mappings…")).toBeVisible();
  expect(screen.queryByLabelText(/Claude mappings/)).not.toBeInTheDocument();
  mocks.error = true;
  rerender(<Page />);
  expect(screen.getByRole("alert")).toHaveTextContent("Could not load");
});
