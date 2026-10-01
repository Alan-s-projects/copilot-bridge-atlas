import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  InputUsageHeading,
  RequestInputValue,
} from "@/components/usage/InputUsage";

const log = {
  appType: "codex",
  inputTokens: 10000,
  freshInputTokens: 1200,
  cacheReadTokens: 8400,
  cacheCreationTokens: 400,
};

describe("request input details", () => {
  it.each([
    [false, "1,200 / 8,400 / 84.0%"],
    [true, "1.2K / 8.4K / 84.0%"],
  ])("shows fresh/cached/hit with compact=%s", (compact, expected) => {
    const { container } = render(
      <RequestInputValue log={log} compact={compact} />,
    );
    expect(container).toHaveTextContent(expected);
    expect(
      screen.getByTitle(
        "Fresh Input: 1,200; Cached Input: 8,400; Read Cache Hit Rate: 84.0%",
      ),
    ).toBeVisible();
  });

  it("labels hit as a rate and does not invent a rate for zero-input requests", () => {
    const { container } = render(
      <>
        <InputUsageHeading />
        <RequestInputValue
          log={{
            ...log,
            freshInputTokens: 0,
            cacheReadTokens: 0,
            cacheCreationTokens: 0,
          }}
        />
      </>,
    );
    expect(screen.getByText("(fresh/cached/hit)")).toBeVisible();
    expect(container).toHaveTextContent("0 / 0 / --");
    expect(screen.getByText("--")).toHaveClass("text-muted-foreground");
  });

  it.each([
    [0, "0.0%", "text-red-700", "dark:text-red-400"],
    [4996, "50.0%", "text-red-700", "dark:text-red-400"],
    [5000, "50.0%", "text-orange-700", "dark:text-orange-400"],
    [7996, "80.0%", "text-orange-700", "dark:text-orange-400"],
    [8000, "80.0%", "text-emerald-700", "dark:text-emerald-400"],
    [10000, "100.0%", "text-emerald-700", "dark:text-emerald-400"],
  ])(
    "colors %i cached tokens using the unrounded rate",
    (cached, text, light, dark) => {
      render(
        <RequestInputValue
          log={{
            freshInputTokens: 10000 - cached,
            cacheReadTokens: cached,
            cacheCreationTokens: 0,
          }}
        />,
      );
      expect(screen.getByText(text)).toHaveClass(light, dark);
    },
  );

  it("counts cache writes as misses when selecting the color", () => {
    render(
      <RequestInputValue
        log={{
          freshInputTokens: 0,
          cacheReadTokens: 400,
          cacheCreationTokens: 600,
        }}
      />,
    );
    expect(screen.getByText("40.0%")).toHaveClass("text-red-700");
  });
});
