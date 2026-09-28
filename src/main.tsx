import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { DatabaseUpgrade } from "./components/DatabaseUpgrade";
import "./index.css";
// Import internationalization configuration
import i18n from "./i18n";
import { QueryClientProvider } from "@tanstack/react-query";
import { ThemeProvider } from "@/components/theme-provider";
import { queryClient } from "@/lib/query";
import { Toaster } from "@/components/ui/sonner";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { message } from "@tauri-apps/plugin-dialog";
import { exit } from "@tauri-apps/plugin-process";
import { FrontendErrorBoundary } from "./components/FrontendErrorBoundary";
import {
  installGlobalErrorHandlers,
  reportFrontendError,
} from "./lib/frontendLogger";
import { initializeWindowActivity } from "@/lib/windowActivity";

installGlobalErrorHandlers();

// Configuration loading error payload type
interface ConfigLoadErrorPayload {
  path?: string;
  error?: string;
  /** An incompatible database renders the recovery view. */
  kind?: string;
}

/**
 * Handle configuration load failure: display error message and force quit app
 * Don't give the user a "cancel" option because the app won't run properly when the configuration is corrupted
 */
async function handleConfigLoadError(
  payload: ConfigLoadErrorPayload | null,
): Promise<void> {
  const path = payload?.path ?? "~/.copilot-bridge-atlas/config.json";
  const detail = payload?.error ?? "Unknown error";

  await message(
    i18n.t("errors.configLoadFailedMessage", {
      path,
      detail,
      defaultValue:
        "Unable to read configuration file:\n{{path}}\n\nError details:\n{{detail}}\n\nPlease check if the JSON is valid, or restore from a backup file (e.g., config.json.bak) in the same directory.\n\nThe app will exit so you can fix this.",
    }),
    {
      title: i18n.t("errors.configLoadFailedTitle", {
        defaultValue: "Configuration Load Failed",
      }),
      kind: "error",
    },
  );

  await exit(1);
}

// Listen to the configuration loading error event of the backend: only remind the user and force exit, without modifying any configuration files
try {
  void listen("configLoadError", async (evt) => {
    await handleConfigLoadError(evt.payload as ConfigLoadErrorPayload | null);
  });
} catch (e) {
  // Ignore event subscription exceptions (e.g. in non-Tauri environments)
  reportFrontendError("config_load_error_listener", e);
}

async function bootstrap() {
  // Start early active query of backend initialization errors to avoid event race conditions
  try {
    const initError = (await invoke(
      "get_init_error",
    )) as ConfigLoadErrorPayload | null;
    if (initError && initError.kind === "db_schema_incompatible") {
      ReactDOM.createRoot(document.getElementById("root")!).render(
        <React.StrictMode>
          <FrontendErrorBoundary>
            <ThemeProvider
              defaultTheme="system"
              storageKey="copilot-bridge-atlas-theme"
            >
              <DatabaseUpgrade payload={initError} />
              <Toaster />
            </ThemeProvider>
          </FrontendErrorBoundary>
        </React.StrictMode>,
      );
      return;
    }
    if (initError && (initError.path || initError.error)) {
      await handleConfigLoadError(initError);
      // Note: It will not be executed here because exit(1) will terminate the process
      return;
    }
  } catch (e) {
    // Ignore pull errors and continue rendering
    reportFrontendError("get_init_error", e);
  }

  initializeWindowActivity();

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <FrontendErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <ThemeProvider
            defaultTheme="system"
            storageKey="copilot-bridge-atlas-theme"
          >
            <App />
            <Toaster />
          </ThemeProvider>
        </QueryClientProvider>
      </FrontendErrorBoundary>
    </React.StrictMode>,
  );
}

void bootstrap();
