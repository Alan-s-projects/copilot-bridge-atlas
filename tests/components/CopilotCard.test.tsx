import { render, screen, within } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CopilotCard } from "@/components/providers/CopilotCard";
import type { Provider } from "@/types";
import { createTestQueryClient } from "../utils/testQueryClient";

const mocks = vi.hoisted(() => ({
  auth: {
    accounts: [{ id: "account-1", login: "test-user" }],
    defaultAccountId: "account-1",
    isLoadingStatus: false,
  },
  quota: vi.fn(),
}));
vi.mock("@/components/providers/forms/hooks/useCopilotAuth", () => ({
  useCopilotAuth: () => mocks.auth,
}));
vi.mock("@/components/CopilotQuotaFooter", () => ({
  default: ({ meta }: { meta: Provider["meta"] }) => {
    mocks.quota(meta);
    return <span>Copilot quota</span>;
  },
}));

const provider: Provider = {
  id: "current-copilot",
  name: "GitHub Copilot",
  settingsConfig: { modelCatalog: { models: [{ model: "gpt-6-astra" }] } },
  meta: {
    providerType: "github_copilot",
    authBinding: {
      authProvider: "github_copilot",
      accountId: "account-1",
    },
  },
};

function renderCard(cardProvider = provider) {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <CopilotCard provider={cardProvider} />
    </QueryClientProvider>,
  );
}

describe("Copilot account card", () => {
  beforeEach(() => {
    mocks.auth.accounts = [{ id: "account-1", login: "test-user" }];
    mocks.auth.isLoadingStatus = false;
    mocks.quota.mockClear();
  });

  it("reports a signed-out account without a health-check action", () => {
    mocks.auth.accounts = [];
    renderCard();
    expect(screen.getByText("Needs setup")).toBeVisible();
    expect(screen.getByText("GitHub Copilot is signed out.")).toBeVisible();
    expect(
      screen.queryByText(/save the bridge settings/),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Health check" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Edit" }),
    ).not.toBeInTheDocument();
    expect(mocks.quota).not.toHaveBeenCalled();
  });

  it("keeps the account, model count, and quota without health-check controls", () => {
    renderCard();
    expect(
      screen.getByText("test-user · 1 models available to Codex"),
    ).toBeVisible();
    expect(screen.queryByText("Needs setup")).not.toBeInTheDocument();
    expect(screen.getByText("Copilot quota")).toBeVisible();
    expect(mocks.quota).toHaveBeenCalledWith(provider.meta);
    const card = screen
      .getByRole("heading", { name: "Provider", level: 2 })
      .closest("section")!;
    expect(
      within(card).getByRole("heading", { name: "GitHub Copilot", level: 3 }),
    ).toBeVisible();
    expect(within(card).queryByRole("button")).not.toBeInTheDocument();
  });

  it("counts only catalog models enabled for Codex", () => {
    renderCard({
      ...provider,
      settingsConfig: {
        modelCatalog: {
          models: [
            { model: "gpt-disabled", enabled: false },
            { model: "gpt-enabled" },
          ],
        },
      },
    });
    expect(
      screen.getByText("test-user · 1 models available to Codex"),
    ).toBeVisible();
  });

  it("asks the user to enable a catalog model when all are disabled", () => {
    renderCard({
      ...provider,
      settingsConfig: {
        modelCatalog: {
          models: [{ model: "gpt-disabled", enabled: false }],
        },
      },
    });
    expect(screen.getByText("Needs setup")).toBeVisible();
    expect(screen.getByText(/No models are enabled for Codex/)).toBeVisible();
  });
});
