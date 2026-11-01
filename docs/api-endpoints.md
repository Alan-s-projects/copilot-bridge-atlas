# Native APIs and Routing

Atlas keeps two local listeners. Routing lists twelve public API entries from the same Rust registry used to register their routes. Diagnostic routes and the Messages model-detail route remain available but are not separate directory entries.

## Responses API listener

| Method | Canonical path | Behavior |
| --- | --- | --- |
| POST | `/v1/responses` | Existing Responses bridge, including per-model upstream conversion |
| POST | `/v1/chat/completions` | Native Chat Completions JSON or SSE |
| POST | `/v1/systemone` | Native Microsoft decision API |
| WS | `/v1/responses` | Native Responses WebSocket events |
| POST | `/v1/responses/compact` | Existing context compaction |
| POST | `/v1/alpha/search` | Existing client search protocol |
| POST | `/v1/images/generations` | Existing image generation passthrough |
| POST | `/v1/images/edits` | Existing JSON image-edit passthrough |
| GET | `/v1/models` | Existing Codex-compatible catalog, not raw Copilot metadata |

New Chat and SystemOne endpoints also accept `/chat/completions` and `/systemone`. Existing Responses, Compact, Search, and Images aliases are preserved. OpenAI `/v1/decisions` is **not** exposed or translated to SystemOne.

### SystemOne

```json
{
  "model": "microsoft-decision-1",
  "state": "I was charged twice.",
  "questions": {
    "billing": {
      "type": "noul",
      "instructions": "Is this a billing issue?"
    }
  }
}
```

The native response contains `answers`, the model identity, and token usage. `state` and question content pass through without private-field stripping. SystemOne is non-streaming and uses a question object, not OpenAI Decisions' question array. Model access is checked against the selected account's live/cached catalog; a hidden decision model is eligible for its dedicated endpoint, not the chat picker.

In Atlas 8.1.1 and later, manage decision-model access under **Settings → Models → Enabled models**. The decision row's switch controls SystemOne access without adding the model to chat selectors. Refresh models after updating if the entry is not yet visible.

### WebSocket

Connect a native WebSocket client to `ws://127.0.0.1:<responses-port>/v1/responses`. Send native `response.create` frames with top-level `model` and `input`, not a nested Realtime API request:

```json
{"type":"response.create","model":"<websocket-capable-model-id>","input":"Reply exactly OK."}
```

Atlas checks `ws:/responses` support for every new response. One response can be active at a time. `response.cancel` cancels the active response. A missing model may inherit only from a successfully completed `previous_response_id` on the same connection. The relay preserves upstream event text and counts terminal usage once per response ID.

Connections pin their account and upstream origin, honor configured outbound HTTP/SOCKS proxy settings, and do not follow redirects or automatically reconnect/replay. Limits: 16 MiB per message, 4,096 completed responses, 4,096 bytes per response ID, an 8 MiB retained-identity budget, one-hour connection lifetime, and five-minute inbound idle timeout. Copilot's rotated created/completed IDs are correlated without changing the forwarded event text. Server stop closes relays. Unsupported binary or request-producing event types fail explicitly.

The new native endpoints accept origin-less native clients. Browser-origin requests are rejected, including WebSocket upgrades, to prevent a webpage from consuming the logged-in account's quota. This is not a public-server authentication scheme: keep Atlas on loopback; do not expose it to an untrusted network.

## Messages API listener

- `POST /v1/messages`
- `POST /v1/messages/count_tokens`
- `GET /v1/models`

These retain the existing Claude-role mappings and behavior. Native SystemOne and Chat routes are not added to this listener. `GET /v1/models/:id` is unchanged.

## Display and pricing

Both endpoint lists and all row details start collapsed. Details contain only a short description and illustrative sample input/output. An available route does not guarantee that every account or model can use it. No model probes run from opening the page.

Decision-1 costs use a [Foundry reference estimate](model-pricing.md), not a confirmed Copilot rate. Source/provenance remains in pricing settings, not endpoint details. Other prices and historical recorded costs are preserved.

## Verification

Normal tests use isolated data and mock upstream servers. Opt-in Rust tests `live_native_copilot_endpoints` and `live_copilot_responses_websocket_one_tiny_response` require explicit temporary `ATLAS_COPILOT_TEST_TOKEN` and `ATLAS_COPILOT_TEST_ENDPOINT`; native Chat also requires `ATLAS_COPILOT_TEST_CHAT_MODEL`. They consume a small amount of real inference usage, send only synthetic input, and do not execute tools. Never commit credentials or include them in test output.
