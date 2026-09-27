import { Loader2, Server } from "lucide-react";
import { Switch } from "@/components/ui/switch";
import { useProxyStatus } from "@/hooks/useProxyStatus";

export function ProxyToggle() {
  const { isRunning, isLoading, isPending, toggleProxy } = useProxyStatus();
  return (
    <div
      className="flex h-8 shrink-0 items-center gap-2 rounded-lg bg-muted/50 px-2"
      title={
        isLoading
          ? "Checking proxy status"
          : isRunning
            ? "Stop proxy"
            : "Start proxy"
      }
    >
      {isLoading || isPending ? (
        <Loader2
          aria-hidden
          className="h-4 w-4 animate-spin text-muted-foreground"
        />
      ) : (
        <Server
          aria-hidden
          className={`h-4 w-4 ${isRunning ? "text-emerald-500" : "text-muted-foreground"}`}
        />
      )}
      <span className="text-xs font-medium">
        {isLoading ? "Checking" : isRunning ? "Proxy Running" : "Proxy Stopped"}
      </span>
      <Switch
        checked={isRunning}
        disabled={isLoading || isPending}
        onCheckedChange={toggleProxy}
        aria-label="Proxy server"
        aria-busy={isLoading || isPending}
      />
    </div>
  );
}
