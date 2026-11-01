# Atlas 8.1.1 validation

## Automated checks

- 821 Rust tests passed; 12 opt-in tests ignored. Tests used disposable application data.
- 315 frontend tests across 58 files passed.
- TypeScript typecheck, renderer build, and 25 validation-script tests passed.
- Rust formatting and diff whitespace checks passed.

## Decision-model behavior

- Management discovery includes a valid, non-policy-disabled SystemOne decision model even when Copilot's chat picker hides it.
- Existing chat eligibility remains unchanged; hidden chat models, embeddings, and unsupported decision protocols remain excluded.
- Refresh preserves `enabled:false` and `modelType` metadata. Reopening the SQLite database and rerunning initialization preserves the toggle.
- One running native router follows saved toggle changes: enabled returns HTTP 200, disabled returns HTTP 400 before forwarding, and reenabled returns HTTP 200 without restart. The test uses a mock upstream, not the user's account.
- Generated Codex JSON excludes both typed and endpoint-identified decision rows, while retaining metadata-less legacy chat rows.
- Claude mapping selectors and backend Messages routing exclude decisions. The provider card's Codex model count excludes them too.

## UI regression checks

The existing chat-row DOM remains identical when a decision row is added. Decision rows alone show Decisions, `/v1/systemone`, and a dedicated enable switch instead of context/reasoning fields.

The actual React form was verified in an isolated renderer preview: default enabled row, disable/save, refresh preserving disabled state, reload preserving state, dark mode, and 375px layout without horizontal overflow. Chat context/reasoning controls remained present and unchanged. The corrected preview had no console errors.

No installed Atlas configuration, real account preference, or client configuration was changed during verification. Installer builds are validated separately by the release workflow.
