export const REQUEST_LOG_COLUMNS = [
  { id: "time", label: "Time" },
  { id: "model", label: "Billing Model" },
  { id: "reasoning", label: "Reasoning (requested/applied)" },
  { id: "status", label: "Status" },
  { id: "input", label: "Input (fresh/cached/hit)" },
  { id: "output", label: "Output" },
  { id: "cacheWrite", label: "Cache Write" },
  { id: "duration", label: "Duration" },
  { id: "pricingTier", label: "Pricing Tier" },
  { id: "cost", label: "Cost" },
] as const;

export const MODEL_STATS_COLUMNS = [
  { id: "model", label: "Billing Model" },
  { id: "requests", label: "Requests" },
  { id: "input", label: "Total Input (fresh/cached/hit)" },
  { id: "output", label: "Total Output" },
  { id: "cacheWrite", label: "Total Cache Write" },
  { id: "cost", label: "Total Cost" },
  { id: "averageCost", label: "Average Cost" },
] as const;

export interface ColumnOption {
  id: string;
  label: string;
}

export function visibleColumns(
  saved: readonly string[] | undefined,
  options: readonly ColumnOption[],
): string[] {
  const known = options.map(({ id }) => id);
  const selected = saved?.filter((id) => known.includes(id));
  return selected?.length ? [...new Set(selected)] : known;
}
