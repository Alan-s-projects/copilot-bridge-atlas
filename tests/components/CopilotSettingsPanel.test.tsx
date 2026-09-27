import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CopilotSettingsPanel } from "@/components/settings/CopilotSettingsPanel";
import type { ProviderFormProps } from "@/components/providers/forms/ProviderForm";
import type { Provider } from "@/types";

const mocks = vi.hoisted(() => ({
  readLive: vi.fn(),
  update: vi.fn(),
  updateTray: vi.fn(),
}));
vi.mock("@/lib/api", () => ({
  providersApi: {
    readLiveSettings: mocks.readLive,
    updateTrayMenu: mocks.updateTray,
  },
}));
vi.mock("@/lib/query", () => ({
  useProvidersQuery: () => ({
    data: {
      providers: { [provider.id]: provider },
      currentProviderId: provider.id,
    },
    isLoading: false,
  }),
  useUpdateProviderMutation: () => ({ mutateAsync: mocks.update }),
}));
vi.mock("@/components/providers/forms/ProviderForm", () => ({
  ProviderForm: ({ initialData, onSubmit, autoSave }: ProviderFormProps) => (
    <>
      <output data-testid="auto-save">{String(autoSave)}</output>
      <output data-testid="settings">
        {JSON.stringify(initialData?.settingsConfig)}
      </output>
      <input aria-label="Model draft" defaultValue="gpt-6-astra" />
      <button
        onClick={() =>
          onSubmit({
            name: "Renamed",
            settingsConfig: JSON.stringify(initialData?.settingsConfig),
            meta: initialData?.meta,
          })
        }
      >
        save
      </button>
    </>
  ),
}));

const provider: Provider = {
  id: "existing-copilot",
  name: "Copilot",
  settingsConfig: {
    config: 'model_provider = "custom"\n# stored template',
    modelCatalog: {
      models: [{ model: "gpt-6-astra", inputModalities: ["text", "image"] }],
    },
  },
  meta: { providerType: "github_copilot", githubAccountId: "account-a" },
};

describe("Copilot provider editing", () => {
  beforeEach(() => {
    mocks.update.mockReset().mockResolvedValue(undefined);
    mocks.updateTray.mockReset().mockResolvedValue(undefined);
  });
  it("preserves stored models and account binding without importing live configuration", async () => {
    render(<CopilotSettingsPanel />);
    expect(screen.getByTestId("auto-save")).toHaveTextContent("true");
    expect(screen.getByTestId("settings")).toHaveTextContent("gpt-6-astra");
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith({
        provider,
      }),
    );
    expect(mocks.readLive).not.toHaveBeenCalled();
    expect(mocks.updateTray).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("settings")).toBeVisible();
  });
});
