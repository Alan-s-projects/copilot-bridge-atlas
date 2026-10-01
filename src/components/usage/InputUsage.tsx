import { getReadCacheHitRate, type InputTokenUsage } from "@/types/usage";
import { fmtInt, formatTokensShort, formatReadCacheHitRate } from "./format";

export function InputUsageHeading({ inline = false }: { inline?: boolean }) {
  return (
    <>
      Input{inline ? " " : null}
      <span
        className={`${inline ? "" : "block "}text-[10px] font-normal text-muted-foreground`}
      >
        (fresh/cached/hit)
      </span>
    </>
  );
}

export function InputUsageValue({
  fresh,
  cached,
  hit,
  hitRate,
  compact = false,
  compactDecimals,
}: {
  fresh: number;
  cached: number;
  hit: string;
  hitRate: number | null;
  compact?: boolean;
  compactDecimals?: 1 | 2;
}) {
  const format = compact
    ? (value: number) => formatTokensShort(value, compactDecimals)
    : (value: number) => fmtInt(value, "en-US");
  const hitColor =
    hitRate == null || !Number.isFinite(hitRate)
      ? "text-muted-foreground"
      : hitRate < 0.5
        ? "text-red-700 dark:text-red-400"
        : hitRate < 0.8
          ? "text-orange-700 dark:text-orange-400"
          : "text-emerald-700 dark:text-emerald-400";
  return (
    <span
      className="whitespace-nowrap tabular-nums"
      title={`Fresh Input: ${fmtInt(fresh, "en-US")}; Cached Input: ${fmtInt(cached, "en-US")}; Read Cache Hit Rate: ${hit}`}
    >
      {format(fresh)}
      <span className="text-muted-foreground"> / </span>
      {format(cached)}
      <span className="text-muted-foreground"> / </span>
      <span className={hitColor}>{hit}</span>
    </span>
  );
}

export function RequestInputValue({
  log,
  compact = false,
}: {
  log: InputTokenUsage;
  compact?: boolean;
}) {
  return (
    <span className="text-xs">
      <InputUsageValue
        fresh={log.freshInputTokens}
        cached={log.cacheReadTokens}
        hit={formatReadCacheHitRate(log)}
        hitRate={getReadCacheHitRate(log)}
        compact={compact}
      />
    </span>
  );
}
