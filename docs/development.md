# Development

Copilot Bridge Atlas is a Windows x64 desktop app connecting Codex to GitHub Copilot. It runs a local OpenAI-compatible server and provides account management, usage statistics, and read-only Codex configuration previews.

User installation and connection steps are in the [README](../README.md).

## Product behavior

Atlas never writes Codex configuration, authentication, MCP, skills, instructions, or conversation files. Connect offers a side-by-side comparison for the bridge and for returning to OpenAI sign-in, with scrolling inside the code pane and copying of the proposed TOML. File selection sits above aligned Connection and Context dropdowns, defaulting to Copilot Bridge and Unchanged. A manual TOML location changes the preview only.

Model names, reasoning levels, and context limits follow Copilot's live metadata. Model enable switches survive app restarts and refresh. Live image and parallel-tool declarations continue to refresh, and Codex's context budget respects the reported input limit. Atlas forwards unsupported-image errors without retrying with images removed.

Connection proposals use the generated per-model catalog for context limits and leave auto-compaction settings unchanged. Changes affect the proposal only.

An upstream HTTP 408 is reported separately from Atlas's own timeout. For repeated request-body timeouts in a long conversation, reduce the context or continue in a new chat with a short handoff. Atlas does not silently remove conversation content or change Codex's compaction settings.

In Usage → Request Logs, failed status codes open a centered detail dialog. Failed requests retain up to 32 KiB each of the outbound Copilot request body and upstream response body, plus up to 8 KiB of headers on each side. A `response.failed` event inside an HTTP 200 stream is saved as the response body and shown with upstream status 200 and history status 502. Long request bodies retain the first and last 16 KiB with an omission marker. Common credential-bearing headers are redacted; bodies and other headers can contain conversation text or credentials. Bounded snapshots are stored in Atlas's local database and written as escaped lines under the same Atlas ID in the file log. Database backups include the snapshots; the usual 30-day request-detail pruning removes their database rows. Treat the file log and backups as sensitive.

### Pages

- **Overview:** Provider, Today's usage, and Requests cards; proxy status, endpoint, quota ring, estimated cost, cache reuse, active requests, and the latest five requests. Connection warnings appear above Provider.
- **Usage:** a compact summary of cost, tokens, and requests, with token and request details, history, grouped cost/rate/token charts, model statistics, and searchable model pricing. Date range, chart grouping, and table column selections auto-save; all three charts share the header's model and date filters. Usage-range warnings identify models without a matching bundled or custom price and include fresh input, output, cache hits, and cache-hit rate. Historical recorded costs are preserved. Imported conversation totals are excluded. Token costs are estimates, not a Copilot subscription bill.
- **Connect:** configuration detection, comparison, copying, and a context-window option.
- **Settings:** appearance/startup, outbound networking, GitHub authentication, unified model catalog, local backups, and About.

Home usage updates are coalesced from request events instead of idle SQL polling. Status and quota polling pause while the window is inactive. Close the window to keep the bridge in the tray; use **Quit** to exit.

The application log is always enabled at Info level and rotates locally. Each
upstream HTTP request records an `atlas_id`, status, elapsed time, endpoint path,
requested/upstream models, transport, streaming and reasoning metadata, request
shape counts, response size, and allowlisted upstream correlation IDs. Failures
also record a bounded diagnostic summary and four escaped request/response
snapshot lines at Warn level. The same `atlas_id` identifies the failed request
in Usage history. Request and response bodies in the file log can include prompts,
tool output, or echoed credentials; handle logs and backups as sensitive data.

Overview and Usage share the same summary component. Request logs, stored costs, and editable per-model prices retain precision. Average latency includes individual requests and weighted daily rollups for the selected range.

Overview's recent-request table shows Input (fresh/cached/hit), followed by Duration
and Cost. Usage's detailed request table retains its existing columns and order,
including Pricing Tier. Shared cache-hit percentages are red below 50%, orange
from 50% to below 80%, and green from 80% upward; unavailable rates stay neutral.
Colors use the unrounded rate, and cache writes still count as cache misses.

Pricing has no models.dev downloads or automatic sync. Local price overrides are stored in `%USERPROFILE%\.copilot-bridge-atlas\model-pricing.json`; the bundled defaults are in `src-tauri/src/resources/model-pricing.json`. They contain 33 entries from GitHub's official Copilot pricing table, including all 11 published long-context tiers. Cost Pricing links to the source, filters model IDs and names as you type, and can reset all overrides to bundled defaults. Resetting removes price overrides and deletion tombstones while preserving recorded history. Custom models without a bundled default become unpriced. Unknown models and distinct vendor variants never borrow another model's price. See [pricing provenance and limitations](model-pricing.md).

Model names, input/total context limits, and reasoning levels are read-only Copilot metadata. Refresh preserves enable switches for models still available. A successful refresh removes missing or ineligible models from the saved list and Codex's generated catalog, including their model preferences; returning models are imported as new. A failed refresh leaves the saved catalog intact. An empty catalog explains when GitHub Copilot must be signed in. Catalog rows can be disabled to hide them from Codex without deleting their settings, pricing, or usage history. Enabled models sort before disabled models, then alphabetically by display name.

Atlas keeps one provider, GitHub Copilot, regardless of the model vendor. Responses or Chat Completions transport is chosen automatically from each model's advertised capabilities; there is no upstream-format setting. Future model IDs using these protocols do not need a code allowlist update. Embedding, completion, hidden, policy-disabled, and unsupported-protocol entries are not exposed as chat models. Explicit refresh fetches a fresh catalog, and routing caches expire after five minutes. Context, output limits, image support, parallel tools, and reasoning metadata remain model-specific.

Model rows show the selected protocol as plain text before the reasoning levels.
Responses takes precedence when both protocols are advertised. The vendor,
image-support, and parallel-tool summary is omitted from the list; those
capabilities continue to drive requests and Codex's generated catalog.

Responses-to-Chat conversion keeps later instructions at their original message
positions instead of merging them into the initial system message. Both the Chat
adapter and the function-only Responses adapter reject opaque compaction items
and unresolved history references before conversion. Ordinary text summaries and
tool-result data remain supported; native Responses forwarding is unchanged.

## Independent application identity

- Process: `copilot-bridge-atlas.exe`
- Application ID: `com.alansprojects.copilotbridgeatlas`
- Default data directory: `%USERPROFILE%\.copilot-bridge-atlas`
- Database: `copilot-bridge-atlas.db`
- Generated catalog: `copilot-model-catalog.json` inside the data directory

The app uses its own installer identity, settings, logs, startup entry, and WebView profile. Atlas 6 creates its database in `%USERPROFILE%\.copilot-bridge-atlas` with its own application ID and schema version 1. Its backups can be restored through Settings → Backup & Restore.

## Develop and release

Requires Windows x64, Node.js, pnpm, the pinned Rust toolchain, Visual Studio C++ Build Tools, and WebView2.

```powershell
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test:unit --maxWorkers=4 --minWorkers=1
./scripts/build-msi.ps1
```

Run Rust tests with isolated application data:

```powershell
$atlasTestHome = Join-Path $env:TEMP ("atlas-tests-" + [guid]::NewGuid().ToString("N"))
$env:COPILOT_BRIDGE_ATLAS_TEST_HOME = $atlasTestHome
cargo test --manifest-path src-tauri/Cargo.toml --locked --offline -- --test-threads=1
```

Run the catalog validation script tests with `python -m unittest discover -s scripts/tests -v`.
The release workflow runs these alongside the frontend and Rust checks.

The renderer and Rust backend ship together: request logs always include
`freshInputTokens`, normalized in Rust using the stored token semantics. Keep
that calculation in the backend. Copilot authentication uses the shared atomic
file writer, and HTTP client initialization or lock errors propagate to callers.
Malformed saved provider JSON is reported instead of being replaced with empty
settings. Protocol adapters and message-ID repairs remain necessary for existing
Codex conversations and Copilot's supported transports.

The local build writes the MSI and SHA256 file to `release/`. A reviewed PR targets `atlas` and includes matching versions in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`, plus `docs/releases/<version>.md`. The Atlas Release workflow builds and checks each PR on Windows. After squash merge, push an `atlas-<version>` tag pointing to the merged commit. The workflow verifies that the tag is on `atlas` and matches the package version, then builds the MSI, verifies its SHA256 file, and publishes both assets with the checked-in release notes. The description begins with the pipeline run, UTC time, branch, and commit. No manual upload is needed. Remove temporary PR branches after merge.

[MIT license and copyright notice](../LICENSE).
