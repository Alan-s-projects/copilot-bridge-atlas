import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { TableColumnsMenu } from "@/components/usage/TableColumnsMenu";
import {
  REQUEST_LOG_COLUMNS,
  visibleColumns,
} from "@/components/usage/tableColumns";

describe("Usage column selection", () => {
  it("defaults to all columns, ignoring missing or invalid selections", () => {
    const all = REQUEST_LOG_COLUMNS.map(({ id }) => id);
    for (const saved of [undefined, [], ["retired-column"]]) {
      expect(visibleColumns(saved, REQUEST_LOG_COLUMNS)).toEqual(all);
    }
    expect(
      visibleColumns(["cost", "cost", "retired"], REQUEST_LOG_COLUMNS),
    ).toEqual(["cost"]);
  });

  it("keeps the multi-select open and can restore all columns", async () => {
    function Menu() {
      const [selected, setSelected] = useState<string[]>(["model", "cost"]);
      return (
        <TableColumnsMenu
          options={REQUEST_LOG_COLUMNS}
          selected={selected}
          onChange={async (next) => setSelected(next)}
        />
      );
    }
    const user = userEvent.setup();
    render(<Menu />);
    await user.click(screen.getByRole("button", { name: "Columns" }));
    await user.click(screen.getByRole("checkbox", { name: "Cost" }));
    expect(
      screen.getByRole("checkbox", { name: "Billing Model" }),
    ).toBeDisabled();
    expect(screen.getByRole("checkbox", { name: "Cost" })).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: "Show all columns" }));
    for (const checkbox of screen.getAllByRole("checkbox"))
      expect(checkbox).toBeChecked();
  });
});
