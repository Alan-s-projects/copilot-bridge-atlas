import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { GlobalProxySettings } from "@/components/settings/GlobalProxySettings";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const mutateAsyncMock = vi.fn();
const testMutateAsyncMock = vi.fn();
const scanMutateAsyncMock = vi.fn();

vi.mock("@/hooks/useGlobalProxy", () => ({
  useGlobalProxyUrl: () => ({
    data: "http://127.0.0.1:7890",
    isLoading: false,
  }),
  useSetGlobalProxyUrl: () => ({
    mutateAsync: mutateAsyncMock,
    isPending: false,
  }),
  useTestProxy: () => ({
    mutateAsync: testMutateAsyncMock,
    isPending: false,
  }),
  useScanProxies: () => ({
    mutateAsync: scanMutateAsyncMock,
    isPending: false,
  }),
}));

describe("GlobalProxySettings", () => {
  beforeEach(() => {
    mutateAsyncMock.mockReset();
    mutateAsyncMock.mockResolvedValue(undefined);
    testMutateAsyncMock.mockReset();
    scanMutateAsyncMock.mockReset();
  });

  it("renders proxy URL input with saved value", async () => {
    render(<GlobalProxySettings />);

    const urlInput = screen.getByPlaceholderText(
      "http://127.0.0.1:7890 / socks5://127.0.0.1:1080",
    );
    // URL objects will have a trailing slash added to them
    await waitFor(() => expect(urlInput).toHaveValue("http://127.0.0.1:7890/"));
  });

  it("auto-saves proxy URL changes without a Save button", async () => {
    render(<GlobalProxySettings />);

    const urlInput = screen.getByPlaceholderText(
      "http://127.0.0.1:7890 / socks5://127.0.0.1:1080",
    );

    fireEvent.change(urlInput, { target: { value: "http://localhost:8080" } });

    expect(
      screen.queryByRole("button", { name: "common.save" }),
    ).not.toBeInTheDocument();

    await waitFor(
      () =>
        expect(mutateAsyncMock).toHaveBeenCalledWith("http://localhost:8080"),
      { timeout: 1500 },
    );
    // When there is no username, the URL is not parsed by the URL object, so there is no trailing slash
  });

  it("flushes a pending edit when the settings panel closes", async () => {
    const { unmount } = render(<GlobalProxySettings />);
    const urlInput = screen.getByPlaceholderText(
      "http://127.0.0.1:7890 / socks5://127.0.0.1:1080",
    );
    fireEvent.change(urlInput, { target: { value: "http://localhost:9090" } });

    unmount();

    await waitFor(() =>
      expect(mutateAsyncMock).toHaveBeenCalledWith("http://localhost:9090"),
    );
  });

  it("clears proxy URL when clear button is clicked", async () => {
    render(<GlobalProxySettings />);

    const urlInput = screen.getByPlaceholderText(
      "http://127.0.0.1:7890 / socks5://127.0.0.1:1080",
    );

    // Wait for initial value to load
    await waitFor(() => expect(urlInput).toHaveValue("http://127.0.0.1:7890/"));

    // Click clear button
    const clearButton = screen.getByTitle("settings.globalProxy.clear");
    fireEvent.click(clearButton);

    expect(urlInput).toHaveValue("");
  });
});
