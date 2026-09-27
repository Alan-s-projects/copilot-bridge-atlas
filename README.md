# Copilot Bridge Atlas

A Windows x64 desktop app connecting **Codex → GitHub Copilot**, with a
local OpenAI-compatible server, account management, usage statistics, and
read-only Codex configuration previews.

## Install and connect

Download `Copilot-Bridge-Atlas-4.2.10-Windows-x64.msi` from
[Releases](https://github.com/Alan-s-projects/copilot-bridge-atlas/releases).
The installer is per-user and unsigned.

1. Open **Settings → Copilot**, sign in to GitHub, and refresh your models.
   Changes save automatically.
2. Turn on the proxy switch. The default address is `http://127.0.0.1:15722/v1`.
3. Open **Connect**, review the proposed TOML, copy it, and apply it yourself.
4. Reload Codex so it loads the selected provider and generated model catalog.

**Atlas never writes Codex configuration, authentication, MCP, skills,
instructions, or conversation files.** Connect offers a side-by-side comparison
for the bridge and for returning to OpenAI sign-in, with scrolling inside the code
pane and copying of the proposed TOML. File selection sits above aligned Connection
and Context dropdowns, defaulting to Copilot Bridge and Unchanged. A manual TOML
location changes the preview only.

Saved reasoning levels and their default survive app restarts and model refresh.
New models inherit Copilot's advertised reasoning defaults. Live image/parallel-tool
declarations continue to refresh, and saved context sizes are capped to the reported
input budget. Atlas forwards unsupported-image errors without retrying with images
removed.

The optional 1M preset proposes a 1,000,000-token context, capped for models with a
smaller saved limit. It leaves auto-compaction settings unchanged. Changes affect
the proposal only.

An upstream HTTP 408 is reported separately from Atlas's own timeout. For repeated
request-body timeouts in a long conversation, reduce the context or continue in a
new chat with a short handoff. Atlas does not silently remove conversation content
or change Codex's compaction settings.

## Pages

- **Overview:** Provider, Proxy, Today's usage, and Requests cards; endpoint, quota,
  estimated cost, cache reuse, and the latest five requests. Connection warnings
  appear above Provider. Health check measures
  endpoint reachability, without an inference or authentication test.
- **Usage:** a compact summary of cost, tokens, and requests, with token and
  request details, history, trends, model statistics, and searchable model pricing.
  Usage-range warnings identify models without a matching bundled or custom
  price and include fresh input, output, cache hits, and cache-hit rate.
  Historical recorded costs are preserved. Imported conversation totals are excluded.
  Token costs are estimates, not a Copilot subscription bill.
- **Connect:** configuration detection, comparison, copying, and a context-window option.
- **Settings:** appearance/startup, outbound networking, GitHub authentication,
  unified model catalog, logs, local backups, and About.

Home usage updates are coalesced from request events instead of idle SQL polling.
Status and quota polling pause while the window is inactive. Close the window to
keep the bridge in the tray; use **Quit** to exit.

The compact Usage Total Cost summary rounds to whole USD and omits a separate
currency label. Overview's estimated daily cost rounds to one decimal place.
Request logs, stored costs, and editable per-model prices retain precision.
Average latency includes individual requests and weighted daily rollups for the
selected range.

Pricing has no models.dev downloads or automatic sync. Existing local price
overrides are stored in `%USERPROFILE%\.copilot-bridge-atlas\model-pricing.json`;
the bundled defaults are in `src-tauri/src/resources/model-pricing.json`.
They contain 33 entries from GitHub's official Copilot pricing table, including
all 11 published long-context tiers. Cost Pricing links to the source, filters model IDs and names as
you type, and can reset all overrides to bundled defaults. Resetting removes
price overrides and deletion tombstones while preserving retired metadata and
recorded history. Custom models without a bundled default become unpriced.
Unknown models and distinct vendor variants never borrow another model's price.
See [pricing provenance and limitations](docs/model-pricing.md).
Existing retired sync metadata remains inactive.
Model names, input/total context limits, and reasoning levels are read-only
Copilot metadata. Refresh replaces legacy overrides while preserving model
enable switches. An empty catalog explains when GitHub
Copilot must be signed in.
Catalog rows can be disabled to hide them from Codex without deleting their
settings, pricing, or usage history. Enabled models sort before disabled models,
then alphabetically by display name. Temporarily unavailable models retain their
saved choices but are omitted from Codex's generated catalog.

Atlas keeps one provider, GitHub Copilot, regardless of the model vendor.
Responses or Chat Completions transport is chosen automatically from each model's
advertised capabilities; there is no upstream-format setting. Future model IDs
using these protocols do not need a code allowlist update. Embedding, completion,
hidden, policy-disabled, and unsupported-protocol entries are not exposed as chat
models. Explicit refresh fetches a fresh catalog, and routing caches expire after
five minutes. Context, output limits, image support, parallel tools, and reasoning
metadata remain model-specific.

## Independent application identity

- Process: `copilot-bridge-atlas.exe`
- Application ID: `com.alansprojects.copilotbridgeatlas`
- Default data directory: `%USERPROFILE%\.copilot-bridge-atlas`
- Database: `copilot-bridge-atlas.db`
- Generated catalog: `copilot-model-catalog.json` inside the data directory

The app uses its own installer identity, settings, logs, startup entry, and WebView
profile. It does not discover or reuse another application's data directory.
Database backups can be restored through Settings → Backup & Restore.
Folder selections saved by older Atlas versions are still honored on startup,
without restoring the removed directory editor or modifying those selections.

## Develop and release

Requires Windows x64, Node.js, pnpm, the pinned Rust toolchain, Visual Studio C++
Build Tools, and WebView2.

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

The local build writes the MSI and SHA256 file to `release/`. Use a reviewed PR
and squash merge into `atlas`, verify that the built source matches the merged
tree, then publish an `atlas-<version>` tag and upload those two explicit files.
There is no cloud build workflow. Only `atlas` remains after temporary PR branches
are removed.

[MIT license and copyright notice](LICENSE).
