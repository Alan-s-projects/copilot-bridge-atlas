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

Select available models in Settings → Models, then configure mappings in
Settings → Routing. Sonnet defaults to the first selected model; empty Opus,
Haiku, and Fable mappings inherit Sonnet. Removed/unavailable targets fail
explicitly instead of silently routing to another model.

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
