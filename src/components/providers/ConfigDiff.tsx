import { cn } from "@/lib/utils";

export interface ConfigDiffLine {
  kind: "context" | "removed" | "added";
  oldLineNumber: number | null;
  newLineNumber: number | null;
  text: string;
}

interface SplitRow {
  current?: ConfigDiffLine;
  proposed?: ConfigDiffLine;
}

function splitRows(lines: ConfigDiffLine[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let removed: ConfigDiffLine[] = [];
  let added: ConfigDiffLine[] = [];
  const flush = () => {
    for (
      let index = 0;
      index < Math.max(removed.length, added.length);
      index++
    ) {
      rows.push({ current: removed[index], proposed: added[index] });
    }
    removed = [];
    added = [];
  };
  for (const line of lines) {
    if (line.kind === "context") {
      flush();
      rows.push({ current: line, proposed: line });
    } else {
      (line.kind === "removed" ? removed : added).push(line);
    }
  }
  flush();
  return rows;
}

function lineStyle(line?: ConfigDiffLine) {
  return line?.kind === "removed"
    ? "bg-red-500/10 text-red-700 dark:text-red-300"
    : line?.kind === "added"
      ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
      : !line
        ? "bg-muted/40"
        : "";
}

function LineText({ line }: { line?: ConfigDiffLine }) {
  return (
    <div className="flex min-w-0">
      <span aria-hidden className="w-5 shrink-0 select-none text-center">
        {line?.kind === "removed" ? "−" : line?.kind === "added" ? "+" : " "}
      </span>
      <pre className="min-w-0 flex-1 whitespace-pre-wrap break-words pr-3 font-mono">
        {line && line.kind !== "context" && (
          <span className="sr-only">
            {line.kind === "removed" ? "Removed: " : "Added: "}
          </span>
        )}
        {line?.text.replace(/\n$/, "") || "\u00a0"}
        {line && !line.text.endsWith("\n") && (
          <span className="block select-none text-[10px] italic text-muted-foreground">
            \ No newline at end of file
          </span>
        )}
      </pre>
    </div>
  );
}

export function ConfigDiff({ lines }: { lines: ConfigDiffLine[] }) {
  const numberStyle =
    "select-none px-2 text-right align-top font-mono text-muted-foreground";
  return (
    <div
      role="region"
      aria-label="Configuration diff"
      tabIndex={0}
      className="code-diff-scrollbar relative min-h-0 flex-1 overflow-x-auto overflow-y-scroll overscroll-contain"
    >
      {lines.length > 0 && (
        <table className="w-full min-w-[640px] table-fixed border-collapse text-xs leading-6">
          <caption className="sr-only">
            Current TOML compared with proposed TOML
          </caption>
          <thead className="sticky top-0 z-10 border-b bg-muted">
            <tr>
              <th
                scope="col"
                className="border-r px-4 py-2 text-left font-medium"
              >
                Current TOML
              </th>
              <th scope="col" className="px-4 py-2 text-left font-medium">
                Proposed TOML
              </th>
            </tr>
          </thead>
          <tbody>
            {splitRows(lines).map((row, index) => (
              <tr key={index}>
                {(["current", "proposed"] as const).map((side) => {
                  const line = row[side];
                  return (
                    <td
                      key={side}
                      className={cn(
                        "p-0 align-top",
                        side === "current" && "border-r",
                        lineStyle(line),
                      )}
                    >
                      <div className="grid grid-cols-[3.5rem_minmax(0,1fr)]">
                        <span aria-hidden className={numberStyle}>
                          {side === "current"
                            ? line?.oldLineNumber
                            : line?.newLineNumber}
                        </span>
                        <LineText line={line} />
                      </div>
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
