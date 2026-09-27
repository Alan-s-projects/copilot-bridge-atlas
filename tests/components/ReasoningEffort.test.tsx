import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReasoningEffortValue } from "@/components/usage/ReasoningEffort";

describe("requested/applied reasoning", () => {
  it.each([
    ["ultra", "max", "ultra / max"],
    ["high", "high", "high / high"],
    ["none", "none", "none / none"],
    ["ultra", undefined, "ultra / \u2014"],
    [undefined, undefined, "\u2014 / \u2014"],
  ])(
    "renders %s / %s without inventing a default",
    (requested, applied, text) => {
      const { container } = render(
        <ReasoningEffortValue
          log={{
            requestedReasoningEffort: requested,
            appliedReasoningEffort: applied,
          }}
        />,
      );
      expect(container).toHaveTextContent(text);
      expect(
        screen.getByTitle(
          `Requested: ${requested || "not specified or recorded"}; Applied: ${applied || "not sent or recorded"}`,
        ),
      ).toBeVisible();
    },
  );
});
