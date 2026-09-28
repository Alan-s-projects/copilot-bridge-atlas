import { useQuery } from "@tanstack/react-query";
import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usageApi } from "@/lib/api/usage";
import type { RequestLog } from "@/types/usage";

export function RequestErrorDialog({
  log,
  onClose,
}: {
  log: RequestLog;
  onClose: () => void;
}) {
  const { data, isLoading, isError } = useQuery({
    queryKey: ["request-diagnostics", log.requestId],
    queryFn: () => usageApi.getRequestDiagnostics(log.requestId),
    staleTime: Infinity,
    retry: false,
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-[48rem] p-0">
        <DialogHeader className="flex-row items-start justify-between gap-3 text-left">
          <div className="space-y-1">
            <DialogTitle className="flex items-center gap-2">
              Request error
              <span className="rounded bg-red-500/10 px-2 py-0.5 text-sm font-medium text-red-700 dark:text-red-400">
                Log status {log.statusCode}
              </span>
            </DialogTitle>
            <DialogDescription>
              Details for this GitHub Copilot request
            </DialogDescription>
          </div>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            aria-label="Close request details"
            className="shrink-0"
            onClick={onClose}
          >
            <X aria-hidden className="h-4 w-4" />
          </Button>
        </DialogHeader>

        <div className="min-h-0 overflow-y-auto p-5 sm:p-6">
          <dl className="mb-5 grid grid-cols-1 gap-x-4 gap-y-3 text-sm sm:grid-cols-2">
            <div>
              <dt className="text-muted-foreground">Model</dt>
              <dd className="break-all font-mono">
                {log.requestModel ?? log.model}
              </dd>
            </div>
            <div>
              <dt className="text-muted-foreground">Time</dt>
              <dd>{new Date(log.createdAt * 1000).toLocaleString()}</dd>
            </div>
            <div className="sm:col-span-2">
              <dt className="text-muted-foreground">Request ID</dt>
              <dd className="break-all font-mono">{log.requestId}</dd>
            </div>
            {data?.upstreamStatus != null && (
              <div>
                <dt className="text-muted-foreground">Upstream status</dt>
                <dd>HTTP {data.upstreamStatus}</dd>
              </div>
            )}
            {data?.failureStage && (
              <div>
                <dt className="text-muted-foreground">Failure stage</dt>
                <dd className="font-mono">{data.failureStage}</dd>
              </div>
            )}
          </dl>

          {isLoading ? (
            <p role="status" className="text-sm text-muted-foreground">
              Loading request and response details…
            </p>
          ) : (
            <>
              {isError && (
                <p role="alert" className="mb-4 text-sm text-destructive">
                  Could not load the recorded request and response details.
                </p>
              )}
              {data ? (
                <div className="grid gap-4 md:grid-cols-2">
                  <section
                    className="min-w-0 rounded-lg border border-border"
                    aria-labelledby="error-response-heading"
                  >
                    <h3
                      id="error-response-heading"
                      className="border-b border-border px-4 py-3 text-sm font-medium"
                    >
                      Response from Copilot
                    </h3>
                    <div className="space-y-3 p-3">
                      <DetailBlock
                        label="Headers"
                        value={data.responseHeaders}
                      />
                      <DetailBlock label="Body" value={data.responseBody} />
                    </div>
                  </section>
                  <section
                    className="min-w-0 rounded-lg border border-border"
                    aria-labelledby="error-request-heading"
                  >
                    <h3
                      id="error-request-heading"
                      className="border-b border-border px-4 py-3 text-sm font-medium"
                    >
                      Request to Copilot
                    </h3>
                    <div className="space-y-3 p-3">
                      <DetailBlock
                        label="Headers"
                        value={data.requestHeaders}
                      />
                      <DetailBlock label="Body" value={data.requestBody} />
                    </div>
                  </section>
                </div>
              ) : !isError ? (
                <p className="text-sm text-muted-foreground">
                  Request and response snapshots were not recorded for this
                  request.
                </p>
              ) : null}
              {log.errorMessage && (
                <section className="mt-4 space-y-2">
                  <h3 className="text-sm font-medium">Recorded diagnostic</h3>
                  <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted/60 p-3 font-mono text-xs">
                    {log.errorMessage}
                  </pre>
                </section>
              )}
              {!data && !log.errorMessage && (
                <p className="mt-4 text-sm text-muted-foreground">
                  No diagnostic was recorded for this request.
                </p>
              )}
              {data && (
                <p className="mt-4 text-xs text-muted-foreground">
                  Failed-request bodies are saved locally up to 32 KiB each and
                  may contain conversation content. The file log records the
                  same bounded snapshots. Common credential headers are
                  redacted.
                </p>
              )}
            </>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

function DetailBlock({
  label,
  value,
}: {
  label: string;
  value?: string | null;
}) {
  return (
    <div className="min-w-0">
      <h4 className="mb-1 text-xs text-muted-foreground">{label}</h4>
      <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted/60 p-3 font-mono text-xs">
        {value ?? "Not recorded for this request."}
      </pre>
    </div>
  );
}
