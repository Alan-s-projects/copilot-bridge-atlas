# Atlas 8.1.0 validation

Verified November 1, 2026 on an Intel Mac with the repository's pinned Rust toolchain and Node runtime.

## Automated checks

- Rust: 816 passed, 0 failed, 12 opt-in tests ignored. Tests used a disposable application-data directory.
- Frontend: 57 files, 310 tests passed; TypeScript typecheck and renderer build passed.
- Build/catalog validation scripts: 25 tests passed.
- Rust formatting and diff whitespace checks passed.
- Package, Cargo, Cargo.lock and Tauri versions agree on 8.1.0.

Coverage includes registry/route parity, native Chat JSON/SSE and SystemOne body preservation, compressed requests, unchanged upstream JSON/text errors and Retry-After, account/model eligibility, browser-origin rejection, pricing precision/overrides/migrations, and WebSocket proxy transport, ID rotation, duplicate/stale events, settings changes, bounds, cancellation, shutdown and exactly-once usage.

## Live Copilot checks

Only synthetic input was sent using an explicitly supplied temporary Copilot token. Credentials were not printed or committed; no tools were executed.

- **SystemOne:** the Atlas native handler forwarded to Copilot and received HTTP 200 with a numerical `noul` result for `microsoft-decision-1`.
- **Responses WebSocket:** the actual relay completed one `gpt-6-luna` request with exactly `OK` and persisted exactly one usage row. The successful ignored test is `live_copilot_responses_websocket_one_tiny_response`.
- **Native Chat:** the route reached Copilot, which returned HTTP 400 `no_available_model_endpoints` for `mai-code-2-flash`, the only native Chat model then advertised to the test account. Atlas preserved that error. Live successful Chat generation was therefore not verified on this account; mock JSON/SSE tests passed. Catalog advertisement does not establish entitlement.

Live verification found that Copilot rotates its opaque response ID between `response.created` and `response.completed`. The relay now correlates the sole active request, retires aliases, and rejects stale events without changing native event text. A regression covers rotation and duplicate terminals during a subsequent response.

## Renderer verification

Verified the actual React Routing component with isolated IPC state and examples exported from the Rust endpoint registry:

- Responses API and Messages API names preserved.
- Both directories start collapsed with nine and three entries.
- Independent per-endpoint expansion and correct sample input/output.
- No removed search/filter toolbar, sample Copy buttons, model lists, or pricing blocks.
- Light and dark modes; 375px layout without horizontal page overflow.
- Running state locks address/port fields; load failure displays an error rather than false counts.
- No browser console errors during the checks.

No installed Atlas instance, user client configuration, or production pricing database was changed. Windows/macOS installer validation is performed by the release workflow before publication.
