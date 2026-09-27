import { ChevronDown, Columns3, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { visibleColumns, type ColumnOption } from "./tableColumns";

export function TableColumnsMenu({
  options,
  selected,
  onChange,
  saving = false,
}: {
  options: readonly ColumnOption[];
  selected?: readonly string[];
  onChange?: (columns: string[]) => Promise<void>;
  saving?: boolean;
}) {
  const visible = visibleColumns(selected, options);
  const change = (next: string[]) => {
    // The persistence hook reports failures and restores the previous choice.
    void onChange?.(next).catch(() => {});
  };
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" disabled={!onChange}>
          {saving ? (
            <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
          ) : (
            <Columns3 aria-hidden className="h-4 w-4" />
          )}
          Columns
          <ChevronDown aria-hidden className="h-3 w-3" />
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-label="Visible columns"
        className="w-72 max-w-[calc(100vw-2rem)] p-2"
      >
        <Button
          size="sm"
          variant="ghost"
          className="mb-1 w-full justify-start"
          disabled={saving || visible.length === options.length}
          onClick={() => change(options.map(({ id }) => id))}
        >
          Show all columns
        </Button>
        <div className="max-h-[min(24rem,60vh)] overflow-y-auto border-t pt-1">
          {options.map(({ id, label }) => {
            const checked = visible.includes(id);
            return (
              <label
                key={id}
                className="flex cursor-pointer items-center gap-2 rounded px-2 py-2 text-sm hover:bg-muted"
              >
                <input
                  type="checkbox"
                  className="h-4 w-4 shrink-0 accent-primary"
                  checked={checked}
                  disabled={saving || (checked && visible.length === 1)}
                  title={
                    checked && visible.length === 1
                      ? "Keep at least one column visible"
                      : undefined
                  }
                  onChange={() =>
                    change(
                      checked
                        ? visible.filter((column) => column !== id)
                        : [...visible, id],
                    )
                  }
                />
                <span>{label}</span>
              </label>
            );
          })}
        </div>
      </PopoverContent>
    </Popover>
  );
}
