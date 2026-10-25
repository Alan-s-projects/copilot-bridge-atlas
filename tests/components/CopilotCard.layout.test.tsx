import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { CopilotCard } from "@/components/providers/CopilotCard";
import type { Provider } from "@/types";
vi.mock("@/components/providers/forms/hooks/useCopilotAuth", () => ({
  useCopilotAuth: () => ({ accounts: [], isLoadingStatus: false }),
}));
vi.mock("@/components/CopilotQuotaFooter", () => ({ default: () => null }));

describe("Provider heading actions", () => {
  it("places the Overview refresh beside Provider without adding a page title", () => {
    render(
      <CopilotCard
        provider={
          { id: "p", name: "GitHub Copilot", settingsConfig: {} } as Provider
        }
        actions={<button>Refresh overview</button>}
      />,
    );
    const heading = screen.getByRole("heading", { name: "Provider" });
    expect(heading.parentElement).toContainElement(
      screen.getByRole("button", { name: "Refresh overview" }),
    );
    expect(
      screen.queryByRole("heading", { name: "Overview" }),
    ).not.toBeInTheDocument();
  });
});
