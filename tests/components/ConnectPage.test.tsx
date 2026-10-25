import { fireEvent, render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ConnectPage } from "@/components/providers/ConnectPage";
vi.mock("@/lib/query/proxy", () => ({
  useGlobalProxyConfig: () => ({
    data: { claude: { address: "127.0.0.1", port: 15723 } },
  }),
}));
vi.mock("@/components/providers/CodexSetupSuggestion", () => ({
  CodexSetupSuggestion: ({ clientNavigation }: any) => (
    <div>Codex TOML preview{clientNavigation}</div>
  ),
}));
vi.mock("@/lib/clipboard", () => ({ copyText: vi.fn() }));
describe("Connect client selection", () => {
  it("keeps Codex as default and shows the unprefixed Claude gateway setup", () => {
    render(<ConnectPage />);
    expect(screen.getByText("Codex TOML preview")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Claude Desktop" }));
    expect(
      screen.getByRole("heading", { name: "Claude Desktop gateway" }),
    ).toBeVisible();
    expect(screen.getByText("http://127.0.0.1:15723")).toBeVisible();
    expect(screen.getByText("atlas-local")).toBeVisible();
    expect(screen.queryByText("Codex TOML preview")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Claude Code" }));
    expect(screen.getByText(/ANTHROPIC_BASE_URL=/)).toHaveTextContent(
      "http://127.0.0.1:15723",
    );
    fireEvent.click(screen.getByRole("button", { name: "Codex" }));
    expect(screen.getByText("Codex TOML preview")).toBeVisible();
  });
});
