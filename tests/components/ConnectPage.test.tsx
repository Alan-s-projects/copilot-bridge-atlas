import { fireEvent, render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { ConnectPage } from "@/components/providers/ConnectPage";
vi.mock("@/lib/query/proxy", () => ({
  useGlobalProxyConfig: () => ({
    data: { claude: { address: "127.0.0.1", port: 15723 } },
  }),
}));
vi.mock("@/components/providers/CodexSetupSuggestion", () => ({
  CodexSetupSuggestion: ({ clientSelector }: any) => (
    <div>Codex TOML preview{clientSelector}</div>
  ),
}));
vi.mock("@/lib/clipboard", () => ({ copyText: vi.fn() }));
describe("Connect client selection", () => {
  it("keeps Codex as default and shows the unprefixed Claude gateway setup", () => {
    render(<ConnectPage />);
    expect(screen.getByText("Codex TOML preview")).toBeVisible();
    fireEvent.change(screen.getByLabelText("Client"), {
      target: { value: "desktop" },
    });
    expect(
      screen.getByRole("heading", { name: "Claude Desktop gateway" }),
    ).toBeVisible();
    expect(screen.getByText("http://127.0.0.1:15723")).toBeVisible();
    expect(screen.getByText("atlas-local")).toBeVisible();
    expect(screen.queryByText("Codex TOML preview")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Client"), {
      target: { value: "cli" },
    });
    expect(screen.getByText(/ANTHROPIC_BASE_URL=/)).toHaveTextContent(
      "http://127.0.0.1:15723",
    );
    fireEvent.change(screen.getByLabelText("Client"), {
      target: { value: "codex" },
    });
    expect(screen.getByText("Codex TOML preview")).toBeVisible();
  });
});
