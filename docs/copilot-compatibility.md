# Copilot Model Switching

Atlas resolves the selected model and endpoint from the authenticated Copilot
catalog. It never changes the requested model as a reasoning fallback.
The model cards show read-only upstream names, identifiers, context limits, and
reasoning levels. Refresh replaces retired overrides; enable switches and the
optional Ultra setting remain user preferences.

## Verified Request Failures

Synthetic live probes on September 27, 2026 reproduced two distinct failures:

- Gemini 3.8 Flash's Chat gateway returns HTTP 400 for a function schema with
  a top-level `anyOf` or `allOf`. The schema works nested under an
  `arguments` property, but the gateway can treat that value as a string.
  Atlas therefore uses an explicit JSON-string envelope, retaining the entire
  original schema in its description for the model and Codex-side validation.
  It unwraps both streamed and JSON replies without changing the tool arguments.
- Grok 4.7 advertises `/responses` but rejects Codex namespace/custom tools with
  HTTP 422 and native `tool_search` with HTTP 400. Atlas adapts xAI's function-only
  dialect using the same tool-name mapping as the Chat bridge. It restores
  namespace, custom-tool, and tool-search identities on replies.

Cross-model readable message/tool history is retained. Foreign encrypted
reasoning is not sent to xAI. Opaque compaction or unresolved item references
cannot be translated losslessly; Atlas returns an actionable local error instead
of silently dropping conversation content.

## Verification

Normal tests use local mock servers and synthetic histories. The ignored
`live_copilot_tool_compatibility` test is opt-in, consumes tokens, and requires
temporary `ATLAS_COPILOT_TEST_TOKEN` and `ATLAS_COPILOT_TEST_ENDPOINT` environment
variables. It sends synthetic greetings and a harmless lookup tool definition,
not private conversation history. It never executes the returned tool.
The patched forwarder passed both streaming and non-streaming calls for Gemini
3.8 Flash and Grok 4.7, including a namespace tool with a root-union schema.

## Codex Reasoning Accounting

For native Responses `gpt-6-astra` and `gpt-6-luna`, Atlas retains Copilot's
reported usage and encrypted reasoning items and supplies the response header
`x-reasoning-included: true` when Copilot omits it. This tells Codex that the
reported input usage already includes replayed encrypted reasoning; it is not a
request header sent to Copilot.

Codex Desktop's bundled CLI `0.158.0-alpha.2.1` can still compact prematurely
before a subsequent turn, including after resume, because that Codex version
clears the prior response's reasoning-accounting state before its pre-turn
compaction check. The same behavior occurs when the header is omitted, so
removing it is not a workaround. This is tracked upstream in
[openai/codex#32483](https://github.com/openai/codex/issues/32483).

Atlas cannot repair the installed Codex runtime. Do not change Atlas usage,
remove encrypted reasoning, or increase a model's prompt limit to hide this
Codex accounting issue. Keep the accounting header and actual model context
budget unchanged; update Codex only after an upstream runtime fix has been
verified for warm turns, resume, and fork behavior.
