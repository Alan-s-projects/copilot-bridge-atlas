import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { EndpointDirectory } from "@/components/proxy/EndpointDirectory";
import { proxyEndpoints } from "../fixtures/proxyEndpoints";

it("uses theme tokens and a narrow stacked / wide side-by-side sample grid", () => {
  const { container } = render(
    <div className="dark">
      <EndpointDirectory
        endpoints={[proxyEndpoints[0]]}
        isPending={false}
        isError={false}
      />
    </div>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Show 1 endpoints" }));
  fireEvent.click(
    screen.getByRole("button", { name: "POST /v1/responses details" }),
  );
  expect(screen.getByText("Available")).toHaveClass("dark:text-emerald-400");
  const sample = screen.getByLabelText("Sample input");
  expect(sample).toHaveClass(
    "bg-muted/50",
    "text-foreground",
    "border-border",
    "overflow-auto",
    "break-words",
  );
  expect(sample.parentElement?.parentElement).toHaveClass(
    "grid-cols-1",
    "md:grid-cols-2",
    "min-w-0",
  );
  expect(container.querySelector(".dark")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: /copy/i }),
  ).not.toBeInTheDocument();
});

it("shows a pending state without a fabricated count", () => {
  render(<EndpointDirectory endpoints={[]} isPending isError={false} />);
  expect(screen.getByRole("status")).toHaveTextContent("Loading endpoints…");
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
