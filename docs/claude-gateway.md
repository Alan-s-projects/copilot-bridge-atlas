# Claude gateway (Atlas 7)

Atlas uses one selected Copilot provider/account and one global running switch,
with two separate listeners:

| Client | Default base URL | API |
| --- | --- | --- |
| Codex | `http://127.0.0.1:15722/v1` | OpenAI Responses |
| Claude Desktop / Code | `http://127.0.0.1:15723` | Anthropic Messages |

The existing Codex routes and Codex model-catalog shape remain on the Responses
listener. The Messages listener exposes `GET /v1/models`, `GET /v1/models/:id`,
`POST /v1/messages`, `POST /v1/messages/count_tokens`, and `GET /health`.
Both sockets bind before the global state becomes Running. If either address is
unavailable, startup fails without leaving a half-started listener. Stop the
proxy before editing addresses/ports. Mapping changes apply to new requests
without restarting listeners. Existing active responses drain on stop.

## Setup and model selection

In Settings → Models, select the GitHub account and refresh **Enabled models**.
Choose an explicit target for each role in **Claude model mapping**, above the
enabled-model list. Sonnet, Opus, Haiku and Fable are independent; there is no
first-model fallback or inherited Sonnet mapping. A numbered badge on Settings
and Models counts missing or unavailable mappings. Existing explicit mappings
are preserved on upgrade; formerly automatic roles need a selection.

Unconfigured roles are omitted from discovery and requests to them fail before
upstream forwarding. Disabled/unavailable targets also fail instead of silently
routing elsewhere. Other configured roles and native Codex requests remain usable.

Desktop model IDs remain Claude-compatible aliases (`claude-sonnet-5`,
`claude-opus-5`, `claude-haiku-4-5`, `claude-fable-5`). Discovery includes each
actual target model as `display_name`; Atlas rewrites the alias before calling
Copilot and records the actual model in usage history. There is no editable
Display name column. The gateway does not edit Claude configuration files.
Desktop's presentation of custom model labels depends on its installed version.

In Claude Desktop's Gateway settings, enter the Messages base URL, choose a
static key such as `atlas-local`, use the `x-api-key` scheme, and enable model
discovery. This local key is a placeholder: Atlas's local listeners do not
validate client API keys. Keep them bound to loopback unless access is otherwise
protected. Never enter your Copilot token in a client. Atlas replaces incoming
credentials with its selected Copilot account for upstream requests.

Claude Code can be launched using the command shown in Connect:

```sh
ANTHROPIC_BASE_URL='http://127.0.0.1:15723' ANTHROPIC_API_KEY='atlas-local' ANTHROPIC_MODEL='claude-sonnet-5' claude
```

## Protocol behavior and limits

Messages are translated to Responses and then use the existing Copilot
transport selector, including its Chat Completions conversion when needed.
JSON and streaming replies are returned in Anthropic format. The adapter
supports system text, images, client-executed tools, tool-result history,
reasoning transport, usage, and Anthropic error envelopes. Hosted web search
is translated when supported by the chosen upstream; other Anthropic server
tools (such as hosted code execution/computer tools) are rejected explicitly.
The gateway does not grant access to models unavailable on the Copilot account.

Token counting is a local estimate, marked by `x-atlas-token-count: estimate`.
Copilot does not expose the exact model tokenizer; image counting is estimated
separately rather than counting base64 bytes. Billing/usage records use the
actual usage returned by Copilot. No 1M context capability is advertised merely
because a Claude alias normally supports it. Client-specific context limits,
prompt instructions, and platform features may differ from native Anthropic.

## Image-history budget

Starting with Atlas 7.1.2, Messages requests have a default-on 50-image safety
budget. This is a conservative gateway setting, not a claim that every Copilot
model has the same upstream limit. Direct Codex Responses requests are unchanged.

The budget runs after Messages-to-Responses conversion and before the shared
forwarder, so ordinary image blocks and supported tool-media wrappers count in
their normalized form. Each image occurrence counts, including repeated images.
Requests within budget retain all their images. Over-budget requests replace
only enough of the oldest eligible tool-result images with explicit text
markers to fit. All message/user images and the latest contiguous batch of user
tool results are protected. Text and tool-call/result identities are preserved.
No client transcript or original image file is edited. The marker is not a
visual summary; the model can no longer inspect an omitted screenshot unless
it is supplied again. Removing earlier content can reduce prompt-cache reuse.

If protected images alone exceed the budget, Atlas returns an actionable local
HTTP 400 before contacting Copilot. Reduce the attachments/latest tool batch, or
start a fresh conversation with a text handoff and essential images. Upstream
image-count HTTP 400s retain their cause and gain recovery guidance; Atlas does
not strip more images and retry. A lower upstream cap, document-derived images,
or the separate 32 MiB Messages request-body limit can still reject a request.
This safeguard is not a general guarantee against all image or size errors.

The existing backend `claude_gateway` setting stores the following optional
`imageHistory` object (camelCase). There is no new UI control:

```json
{
  "imageHistory": {
    "enabled": true,
    "maxImages": 50,
    "modelLimits": {}
  }
}
```

Omitted settings use these defaults, including upgrades from older versions.
`modelLimits` maps actual Copilot model IDs, not Claude role aliases, to positive
image budgets; for example `{"gpt-6-astra": 50}`. An unlisted model uses
`maxImages`. Set `enabled` to `false` to retain existing forwarding behavior.
These fields are part of the existing backend global-proxy configuration API
and are not Anthropic request parameters. Existing configuration persistence
and runtime updates apply without a schema migration or socket restart.

For requests containing images, the application log records the model, whether
the policy is enabled, the input/protected/omitted/forwarded image counts, and
the budget; these additional log fields contain no image data. Existing request
snapshot logging remains subject to its documented sensitive-data limitations.
`/v1/messages/count_tokens` uses the same selection and estimates the converted
instructions, input, and tools. Each retained image contributes 2,048 estimated
tokens, regardless of dimensions; the result is not an exact tokenizer or bill.

## Context-overflow recovery

The Messages gateway translates genuine upstream input-context errors into HTTP
400 `invalid_request_error` responses with `prompt is too long` wording. Streamed
failure events use the same Anthropic error type and message. Valid upstream
counts are preserved as `prompt is too long: N tokens > M maximum`; missing or
unusable counts are omitted, not estimated. Original failures remain available
through the existing bounded diagnostics.

This lets Claude Code recognize an oversized prompt and attempt its own
compact-and-retry recovery. Atlas does not summarize or trim the conversation,
inflate usage, change model mappings, or write Claude compaction settings. A
successful output stopped at its context/output limit is not reclassified as an
input error. Other validation, image, authentication, rate-limit, payload-size,
and server failures retain their existing behavior.

Recovery is reactive: it does not lower Claude Code's proactive compaction
threshold or guarantee that an already-oversized summarization request will fit.

### Real Claude Code verification

The ignored `context_overflow_real_claude` tests run an installed Claude Code
binary against actual Atlas routes with a loopback Copilot fixture. They check
for `compact_boundary` with automatic recovery and a successful continuation,
and include a control using the previous error wording. Claude home, settings,
and transcripts are isolated; no compaction settings are overridden. Set
`ATLAS_TEST_CLAUDE_BIN` to the executable and `ATLAS_TEST_CLAUDE_ARTIFACTS` to an
absolute scratch directory, then run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --locked context_overflow_real_claude -- --ignored --nocapture --test-threads=1
```

The separate `context_overflow_live_claude_compacts_and_retries` test additionally
requires `ATLAS_LIVE_AUTH_DIR`, containing local `copilot_auth.json`. It uses a
disposable copy of authentication and sends a small synthetic conversation to
Copilot. Only the context-overflow rejection is injected; the initial reply,
summary, and continuation are real GPT-6 Astra responses. This spends Copilot
usage and must be run explicitly. Both variants passed with Claude Code 2.1.308;
neither constitutes a measurement of the upstream's maximum accepted context.

## Reused implementation and validation

The isolated `proxy/claude_protocol` converters, reasoning transport, media
helpers, and streaming tests are recovered from commit
`a7c63c00efea4fb5b72c5194700eb84ac756b049`, before the Codex-only cleanup.
Only the conversion code was recovered; the old multi-provider app and client
file writers were not restored. The existing Copilot authentication, routing,
usage, diagnostics, and Codex pipeline remain the common forwarding path.

Offline tests cover both socket contracts, collisions, shutdown, mapping
persistence, Chat/Responses transports, JSON/SSE, images, tool histories, and
error translation. The opt-in live smoke test uses a temporary copy of local
Copilot authentication and checks a real Claude-style tool-call/result exchange
plus a native Codex Responses request. It is ignored in CI by default.
Native Claude Desktop UI and model-label presentation require validation with
the user's installed client; the automated tests validate the HTTP contracts.

## Client identity

Usage identifies both Claude Desktop's native Electron requests (connection
checks and automatic titles) and its claude-cli agent requests as Claude Desktop.
Generic SDK or browser signatures alone are not enough to attribute another app.
