import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutSection } from "@/components/settings/AboutSection";

const mocks = vi.hoisted(() => ({
  getVersion: vi.fn(),
  openExternal: vi.fn(),
  checkUpdates: vi.fn(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: mocks.getVersion,
}));
vi.mock("@/lib/api", () => ({
  settingsApi: {
    openExternal: mocks.openExternal,
    checkUpdates: mocks.checkUpdates,
  },
}));

beforeEach(() => {
  mocks.getVersion.mockReset().mockResolvedValue("6.0.5");
  mocks.openExternal.mockReset().mockResolvedValue(undefined);
  mocks.checkUpdates.mockReset().mockResolvedValue(undefined);
});

describe("AboutSection release reminder", () => {
  it("shows the shared release result above the About card without another check", async () => {
    render(<AboutSection availableReleaseVersion="6.0.6" />);

    const reminder = await screen.findByRole("status");
    expect(reminder).toHaveTextContent("New release available");
    expect(reminder).toHaveTextContent("Atlas 6.0.6");
    const about = await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 6.0.5",
    });
    expect(
      reminder.compareDocumentPosition(about) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "View release" }));
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://github.com/Alan-s-projects/copilot-bridge-atlas/releases/tag/atlas-6.0.6",
    );
    expect(reminder).toBeVisible();
  });

  it.each([null, undefined])(
    "keeps About usable without a reminder when the shared result is %s",
    async (availableReleaseVersion) => {
      render(
        <AboutSection availableReleaseVersion={availableReleaseVersion} />,
      );
      await screen.findByRole("heading", {
        name: "Copilot Bridge Atlas 6.0.5",
      });
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    },
  );

  it("keeps the GitHub link without the old release and MSI buttons", async () => {
    render(<AboutSection />);
    await screen.findByRole("heading", {
      name: "Copilot Bridge Atlas 6.0.5",
    });
    expect(screen.getByRole("button", { name: "GitHub" })).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Release notes" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Download MSI" }),
    ).not.toBeInTheDocument();
    expect(mocks.checkUpdates).not.toHaveBeenCalled();
  });
});
