import { useId, useState } from "react";
import { ChevronRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ProxyEndpoint } from "@/types/proxy";

function EndpointRow({ endpoint }: { endpoint: ProxyEndpoint }) {
  const [expanded, setExpanded] = useState(false);
  const detailId = useId();
  return (
    <li className="min-w-0">
      <button
        type="button"
        aria-expanded={expanded}
        aria-controls={detailId}
        aria-label={`${endpoint.method} ${endpoint.path} details`}
        onClick={() => setExpanded(!expanded)}
        className="grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-2 rounded-md px-2 py-3 text-left hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:grid-cols-[auto_minmax(0,1fr)_auto_auto]"
      >
        <span className="rounded bg-muted px-2 py-1 font-mono text-xs font-medium text-foreground">
          {endpoint.method}
        </span>
        <code className="break-all text-sm">{endpoint.path}</code>
        <span
          className={`col-start-2 row-start-2 w-fit rounded-full px-2 py-0.5 text-xs sm:col-start-3 sm:row-start-1 ${
            endpoint.implemented
              ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-400"
              : "bg-muted text-muted-foreground"
          }`}
        >
          {endpoint.implemented ? "Available" : "Not implemented"}
        </span>
        <ChevronRight
          aria-hidden="true"
          className={`col-start-3 row-start-1 h-4 w-4 text-muted-foreground sm:col-start-4 ${expanded ? "rotate-90" : ""}`}
        />
      </button>
      <div
        id={detailId}
        hidden={!expanded}
        className="space-y-3 px-2 pb-4 pt-1"
      >
        <p className="text-sm text-muted-foreground">{endpoint.description}</p>
        <div className="grid min-w-0 grid-cols-1 gap-4 md:grid-cols-2">
          {[
            ["Sample input", endpoint.sampleInput],
            ["Sample output", endpoint.sampleOutput],
          ].map(([label, sample]) => (
            <div key={label} className="min-w-0 space-y-2">
              <h4 className="text-xs font-medium text-muted-foreground">
                {label}
              </h4>
              <pre
                aria-label={label}
                tabIndex={0}
                className="max-h-60 overflow-auto whitespace-pre-wrap break-words rounded-lg border border-border bg-muted/50 p-3 text-xs leading-relaxed text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring md:h-60"
              >
                <code>{sample}</code>
              </pre>
            </div>
          ))}
        </div>
      </div>
    </li>
  );
}

export function EndpointDirectory({
  endpoints,
  isPending,
  isError,
}: {
  endpoints: ProxyEndpoint[];
  isPending: boolean;
  isError: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  return (
    <div className="min-w-0 space-y-3 border-t border-border pt-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-medium">Exposed endpoints</h3>
        {!isPending && !isError && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-expanded={expanded}
            aria-controls={listId}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? "Hide endpoints" : `Show ${endpoints.length} endpoints`}
          </Button>
        )}
      </div>
      {isPending ? (
        <p role="status" className="text-sm text-muted-foreground">
          Loading endpoints…
        </p>
      ) : isError ? (
        <p role="alert" className="text-sm text-destructive">
          Could not load endpoints.
        </p>
      ) : (
        <div id={listId} hidden={!expanded}>
          {endpoints.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No endpoints available.
            </p>
          ) : (
            <ul className="min-w-0 divide-y divide-border">
              {endpoints.map((endpoint) => (
                <EndpointRow key={endpoint.id} endpoint={endpoint} />
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
