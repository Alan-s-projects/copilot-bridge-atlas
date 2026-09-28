import { error as writeErrorLog } from "@tauri-apps/plugin-log";

const MAX_LOG_MESSAGE_LENGTH = 12_000;
const MAX_RAW_LOG_INPUT_LENGTH = 16_000;
const MAX_SERIALIZED_STRING_LENGTH = 2_000;
const MAX_SERIALIZED_ENTRIES = 32;
const MAX_SERIALIZED_TOTAL_VALUES = 64;
const MAX_SERIALIZATION_DEPTH = 4;
const QUERY_VALUE_PATTERN = /([?&][A-Za-z0-9_.~-]+)=([^&#\s"'<>]*)/g;
const URL_CREDENTIAL_PATTERN = /(https?:\/\/)[^/@\s]+@/gi;
const QUOTED_NAMED_SECRET_PATTERN =
  /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|authorization|auth|password|passwd|pwd|secret|cookie)\s*["']?\s*[:=]\s*)(["'])(.*?)\2/gi;
const NAMED_SECRET_PATTERN =
  /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|token|authorization|auth|password|passwd|pwd|secret|cookie)\s*["']?\s*[:=]\s*["']?)([^\s"',}]+)/gi;
// When the value of the sensitive key is an array/object (`"tokens":[...]`, `"auth":{...}`), the scalar regular expression cannot reach the elements inside.
// The text layer is the only outlet where all entries (Error/string/object/nested/prefix+JSON) finally converge, so here
// Tip: After hitting the sensitive key name, replace the entire `[..]`/`{..}` that follows. `\b` prevents matching suffixes such as monkey,
// `(?:\\?["'])?` is compatible with both bare quotes and escaped quotes (`\"tokens\"` in double-encoded JSON).
const NAMED_SECRET_CONTAINER_PATTERN =
  /((?:\\?["'])?\b(?:api[_-]?key|access[_-]?key|secret[_-]?key|private[_-]?key|client[_-]?secret|auth[_-]?token|access[_-]?token|refresh[_-]?token|id[_-]?token|session[_-]?token|session[_-]?id|authorization|credential|password|passwd|bearer|cookie|secret|token|auth|pwd|key)s?(?:\\?["'])?\s*[:=]\s*)(\[[^\]]*\]|\{[^{}]*\})/gi;
const SENSITIVE_HEADER_LINE_PATTERN =
  /(^|[\r\n])([ \t]*(?:(?:proxy-)?authorization|cookie|set-cookie|x-api-key|api-key)\s*[:=]\s*)[^\r\n]+/gim;
const AUTH_SCHEME_PATTERN =
  /\b(Bearer|Basic|Token|ApiKey|Digest|Negotiate|AWS4-HMAC-SHA256)\s+[^\s"',}\]]+/gi;
const SECRET_VALUE_IN_TEXT_PATTERN =
  /(^|[^A-Za-z0-9]|\\[nrt])(?:sk-[A-Za-z0-9._~+\/-]{6,}|AIza[A-Za-z0-9_-]{8,}|github_pat_[A-Za-z0-9_]{6,}|gh[pousr]_[A-Za-z0-9_]{6,}|xox[baprs]-[A-Za-z0-9-]{6,}|ya29\.[A-Za-z0-9._-]{6,}|(?:AKIA|ASIA)[A-Z0-9]{12,}|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)/gim;
const SECRET_VALUE_WITHIN_STRING_PATTERN =
  /(?:sk-[A-Za-z0-9._~+\/-]{6,}|AIza[A-Za-z0-9_-]{8,}|github_pat_[A-Za-z0-9_]{6,}|gh[pousr]_[A-Za-z0-9_]{6,}|xox[baprs]-[A-Za-z0-9-]{6,}|ya29\.[A-Za-z0-9._-]{6,}|(?:AKIA|ASIA)[A-Z0-9]{12,}|eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+)/i;
function truncateForProcessing(input: string, limit: number): string {
  if (input.length <= limit) {
    return input;
  }
  return `${input.slice(0, limit)}\n[input truncated]`;
}

export function redactFrontendLogText(input: string): string {
  return input
    .replace(QUERY_VALUE_PATTERN, "$1=[REDACTED]")
    .replace(URL_CREDENTIAL_PATTERN, "$1[REDACTED]@")
    .replace(SENSITIVE_HEADER_LINE_PATTERN, "$1$2[REDACTED]")
    .replace(AUTH_SCHEME_PATTERN, "$1 [REDACTED]")
    .replace(SECRET_VALUE_IN_TEXT_PATTERN, "$1[REDACTED]")
    .replace(NAMED_SECRET_CONTAINER_PATTERN, "$1[REDACTED]")
    .replace(QUOTED_NAMED_SECRET_PATTERN, "$1$2[REDACTED]$2")
    .replace(NAMED_SECRET_PATTERN, "$1[REDACTED]");
}

function looksLikeSecretValue(value: string): boolean {
  const trimmed = value.trim();
  if (SECRET_VALUE_WITHIN_STRING_PATTERN.test(trimmed)) {
    return true;
  }
  if (/-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/i.test(trimmed)) {
    return true;
  }

  // Unknown opaque credentials: keep this only in the structured serializer,
  // where a false positive costs diagnostics but cannot alter application data.
  return (
    trimmed.length >= 32 &&
    /^[A-Za-z0-9._~+/=-]+$/.test(trimmed) &&
    /[A-Za-z]/.test(trimmed) &&
    /\d/.test(trimmed)
  );
}

// In structured objects, hitting these attribute names will determine that the entire value (scalar/array/object) is sensitive and hidden as a whole.
// In the form of depositing single numbers, remove the trailing s before looking up the table, so that the plural numbers (tokens/apiKeys/credentials) will be automatically overwritten.
// No need to enumerate one by one - regular text layers can only match `"name":"value"` scalars, not arrays/nested ones.
const SENSITIVE_KEY_NAMES = new Set([
  "key",
  "apikey",
  "accesskey",
  "secretkey",
  "privatekey",
  "clientsecret",
  "token",
  "authtoken",
  "accesstoken",
  "refreshtoken",
  "idtoken",
  "sessiontoken",
  "sessionid",
  "authorization",
  "auth",
  "bearer",
  "password",
  "passwd",
  "pwd",
  "secret",
  "credential",
  "cookie",
]);

function isSensitiveKey(key: string): boolean {
  const normalized = key
    .toLowerCase()
    .replace(/[^a-z0-9]/g, "")
    .replace(/s$/, "");
  return SENSITIVE_KEY_NAMES.has(normalized);
}

function normalizeForSerialization(
  value: unknown,
  depth: number,
  ancestors: WeakSet<object>,
  budget: { remaining: number },
): unknown {
  if (budget.remaining <= 0) {
    return "[Serialization budget exhausted]";
  }
  budget.remaining -= 1;

  if (typeof value === "string") {
    // Value-level desensitization: Hide the entire opaque string that "looks like a key". Named fields (apiKey/token/...)
    // The upper layer isSensitiveKey is hidden as a whole according to the attribute name; the naked key in the text is then hidden by redactFrontendLogText.
    if (looksLikeSecretValue(value)) {
      return "[REDACTED]";
    }
    return truncateForProcessing(value, MAX_SERIALIZED_STRING_LENGTH);
  }
  if (
    value == null ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (typeof value === "bigint") {
    return `${value}n`;
  }
  if (typeof value === "symbol") {
    return String(value);
  }
  if (typeof value === "function") {
    return `[Function ${value.name || "anonymous"}]`;
  }
  if (typeof value !== "object") {
    return String(value);
  }
  if (ancestors.has(value)) {
    return "[Circular]";
  }
  if (depth >= MAX_SERIALIZATION_DEPTH) {
    return "[Object: max depth reached]";
  }

  ancestors.add(value);
  try {
    if (Array.isArray(value)) {
      const items: unknown[] = [];
      for (const item of value.slice(0, MAX_SERIALIZED_ENTRIES)) {
        if (budget.remaining <= 0) {
          break;
        }
        items.push(
          normalizeForSerialization(item, depth + 1, ancestors, budget),
        );
      }
      if (value.length > items.length) {
        items.push(`[${value.length - items.length} more items]`);
      }
      return items;
    }

    const output: Record<string, unknown> = {};
    const keys = Object.keys(value);
    for (const key of keys.slice(0, MAX_SERIALIZED_ENTRIES)) {
      if (budget.remaining <= 0) {
        output["[truncated]"] = "Serialization budget exhausted";
        break;
      }
      try {
        const descriptor = Object.getOwnPropertyDescriptor(value, key);
        if (descriptor?.get) {
          output[key] = "[Getter omitted]";
          continue;
        }
        if (isSensitiveKey(key)) {
          // Sensitive attribute names → The entire value (including arrays/objects) is hidden, no recursion, no shape guessing.
          output[key] = "[REDACTED]";
          continue;
        }
        output[key] = normalizeForSerialization(
          descriptor?.value,
          depth + 1,
          ancestors,
          budget,
        );
      } catch {
        output[key] = "[Property access failed]";
      }
    }
    if (keys.length > MAX_SERIALIZED_ENTRIES) {
      output["[truncated]"] =
        `${keys.length - MAX_SERIALIZED_ENTRIES} more properties`;
    }
    return output;
  } finally {
    ancestors.delete(value);
  }
}

function serializeStructured(value: unknown): string | null {
  try {
    const serialized = JSON.stringify(
      normalizeForSerialization(value, 0, new WeakSet(), {
        remaining: MAX_SERIALIZED_TOTAL_VALUES,
      }),
    );
    return serialized ?? null;
  } catch {
    return null;
  }
}

// Structured data may be mixed in as "JSON in string form" (Promise.reject(JSON.stringify(...)),
// throw new Error(JSON.stringify(...))). Without restoring the structure, only text regularization is left, and arrays/nested fields are out of reach.
// If the JSON structure is hit, attribute-level desensitization will be performed after parse; if it is not JSON, null will be returned and the interaction caller will process it as ordinary text.
function redactStructuredString(text: string): string | null {
  const trimmed = text.trim();
  if (!(trimmed.startsWith("{") || trimmed.startsWith("["))) {
    return null;
  }
  if (text.length > MAX_RAW_LOG_INPUT_LENGTH) {
    // Once the legal oversized JSON is truncated, it will become illegal JSON, and the text regularity that cannot reach the array field will be returned and leaked;
    // Nor does it risk parse blocking the UI with multiple MiB inputs. If JSON-like is checked out and exceeds the limit, the entire file will be discarded.
    return "[oversized structured error omitted]";
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return null;
  }
  // Scalar JSON (such as "42") has no fields to remove and is given to the text layer.
  if (parsed === null || typeof parsed !== "object") {
    return null;
  }
  return serializeStructured(parsed) ?? "[Unserializable structured error]";
}

// Render Error into "desensitized message + native call stack", regardless of the stack format of the browser engine:
//  - V8/Chromium (Windows WebView2): embedded message in the first line of stack → replace global literals with desensitized versions;
//  - WebKit/JSC (macOS/Linux WKWebView), SpiderMonkey: stack is a pure stack frame without message → desensitization header.
// Does not recognize engine-specific formats such as `    at ` / `@` (the enumeration is not complete, it is the entire WebKit stack that is lost), just press
// "Whether the message appears in the stack" is separated, so that each platform retains the native call stack and no undesensitized messages remain.
function renderRedactedError(error: Error, structuredMessage: string): string {
  const head = `${error.name}: ${structuredMessage}`;
  const stack = error.stack;
  if (!stack) {
    return head;
  }
  if (error.message && stack.includes(error.message)) {
    // V8: message is embedded in the stack - global literal replacement (split/join replaces all occurrences), and the stack frame is retained as is.
    return stack.split(error.message).join(structuredMessage);
  }
  // WebKit/Firefox: stack does not contain message - a desensitized message header is prepended to the pure stack frame.
  return `${head}\n${stack}`;
}

function describeError(error: unknown): string {
  if (error instanceof Error) {
    // throw new Error(JSON.stringify(payload)) is very common: the credentials will be hidden in the message, and V8's
    // The first line of the stack is the original message. First desensitize the message according to the JSON structure, and render it into
    // "Desensitized message + native stack" prevents the stack from spitting out undesensitized messages directly.
    const structuredMessage = redactStructuredString(error.message);
    if (structuredMessage !== null) {
      return truncateForProcessing(
        renderRedactedError(error, structuredMessage),
        MAX_RAW_LOG_INPUT_LENGTH,
      );
    }
    return truncateForProcessing(
      error.stack || `${error.name}: ${error.message}`,
      MAX_RAW_LOG_INPUT_LENGTH,
    );
  }
  if (typeof error === "string") {
    const structured = redactStructuredString(error);
    if (structured !== null) {
      return structured;
    }
    return truncateForProcessing(error, MAX_RAW_LOG_INPUT_LENGTH);
  }
  if (error == null) {
    return String(error);
  }
  const structured = serializeStructured(error);
  if (structured === null) {
    return "[Unserializable thrown value]";
  }
  return truncateForProcessing(structured, MAX_RAW_LOG_INPUT_LENGTH);
}

export function reportFrontendError(
  context: string,
  error: unknown,
  details?: string,
): void {
  // First limit each piece of original input, and then perform global regularization to avoid blocking the UI when exceptions carry multiple MiB text.
  const raw = truncateForProcessing(
    [
      `[frontend] ${truncateForProcessing(context, MAX_RAW_LOG_INPUT_LENGTH)}`,
      describeError(error),
      details
        ? truncateForProcessing(details, MAX_RAW_LOG_INPUT_LENGTH).trim()
        : undefined,
    ]
      .filter(Boolean)
      .join("\n"),
    MAX_RAW_LOG_INPUT_LENGTH,
  );
  const redacted = redactFrontendLogText(raw);
  const message =
    redacted.length > MAX_LOG_MESSAGE_LENGTH
      ? `${redacted.slice(0, MAX_LOG_MESSAGE_LENGTH)}\n[truncated]`
      : redacted;

  // The web development/test environment does not have Tauri invoke, and log reporting failure should no longer be triggered.
  // console.error or the Promise is not handled, otherwise an error loop will form.
  void writeErrorLog(message, { file: "frontend" }).catch(() => undefined);
}

export function installGlobalErrorHandlers(
  target: Window = window,
): () => void {
  const handleError = (event: ErrorEvent) => {
    const location = event.filename
      ? `${event.filename}:${event.lineno}:${event.colno}`
      : undefined;
    reportFrontendError("window.error", event.error ?? event.message, location);
  };
  const handleUnhandledRejection = (event: PromiseRejectionEvent) => {
    reportFrontendError("unhandledrejection", event.reason);
  };

  target.addEventListener("error", handleError);
  target.addEventListener("unhandledrejection", handleUnhandledRejection);

  return () => {
    target.removeEventListener("error", handleError);
    target.removeEventListener("unhandledrejection", handleUnhandledRejection);
  };
}
