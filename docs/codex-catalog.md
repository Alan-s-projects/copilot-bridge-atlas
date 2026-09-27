# Atlas Codex Catalog

Atlas combines Copilot capabilities with unchanged OpenAI Codex instruction data.
The instruction source is pinned to the user-selected OpenAI commit
`d8ec479c34895214b44c062f060d97224c191a50`, file
`codex-rs/models-manager/models.json`. Generation never reads `models_cache.json`
or fetches a newer prompt at application startup.

| Generated model ID | OpenAI instruction entry |
| --- | --- |
| `gpt-6-astra` | `gpt-6-astra` |
| `gpt-6-luna` | `gpt-6-luna` |
| `gpt-6-sol` | `gpt-6-sol` |
| Every other ID, including non-GPT models | `gpt-6-astra` |

Matching is case-insensitive for these exact GPT-6 IDs. The fallback deliberately
retains OpenAI's GPT-6 identity wording, as requested. It does not change the
actual model ID, provider, routing, enabled choices, or billing model.

## Sources

- `src-tauri/src/resources/codex-model-template.json` contains the small,
  model-neutral Codex compatibility profile.
- `src-tauri/src/resources/openai-codex-instructions.json` contains the three
  complete, unchanged upstream `model_messages` objects, source provenance,
  and SHA-256 fingerprints. The former Atlas-authored prompt is removed.
- Model identity, context limits, parallel calls, image input, and supported
  reasoning efforts come from the Copilot capabilities stored by catalog refresh.
  The optional existing Ultra mapping remains unchanged.

The generator copies the entire selected `model_messages` object, not only its
opening template. Persistent, approval, collaboration, agent, confirmation-policy,
and other instruction sections remain as upstream supplied them, including nulls.
It mirrors `instructions_template` into the legacy `base_instructions` field,
matching OpenAI's legacy serialization approach.

No instruction text is trimmed, reworded, appended, or rendered by Atlas.
Trailing newlines and literal `{{connector_id}}` examples are preserved.
Compatibility metadata cannot inject a second set of instruction overrides.

`scripts/vendor-openai-instructions.py` performs the mechanical extraction.
Run it with `--check` to compare the snapshot against the exact remote commit
without writing files. Tests also verify prompt and whole-object fingerprints.
Attribution and Apache-2.0 license/NOTICE content are retained in
`THIRD_PARTY_NOTICES.md` and `BUNDLED_LICENSES.txt`; the latter is distributed
in the MSI's `LICENSE.rtf`.

Atlas explicitly selects shell-command and freeform-patch compatibility. Tool
output truncation and the effective context budget are client-side policies.
Reasoning summaries, verbosity control, original-detail images, and native search
are not advertised based on another model's flags. Missing image and parallel-call
declarations stay conservative. Missing reasoning declarations expose `none`.
The existing 128K context fallback applies only when neither model metadata nor
an explicit context value is available.

The instruction change affects Atlas's generated catalog only. It neither
modifies global Codex configuration nor adds another settings switch.

## Installed-Client Checks

The external catalog was accepted by both installed clients on September 27, 2026:

- Codex Desktop 26.924.2738.0, bundled CLI `0.158.0-alpha.2.1`.
- Standalone CLI `0.146.1`.

Checks use isolated `CODEX_HOME` and temporary workspaces. They do not read or
rewrite the user's Codex configuration. Windows test sessions explicitly use the
unelevated workspace-write sandbox, not unrestricted execution.

To prepare a fixture from an existing Atlas catalog, use:

```powershell
python scripts/check-codex-catalog.py --source-catalog <current-catalog.json> --prepare-settings <settings-fixture.json>
$env:ATLAS_CODEX_CATALOG_SETTINGS = "<settings-fixture.json>"
$env:ATLAS_CODEX_CATALOG_OUT = "<generated-catalog.json>"
cargo test --manifest-path src-tauri/Cargo.toml --lib export_catalog_compatibility_fixture -- --ignored
python scripts/check-codex-catalog.py --codex <codex.exe> --catalog <generated-catalog.json> --capture --report <capture-report.json>
python scripts/check-codex-catalog.py --codex <codex.exe> --catalog <generated-catalog.json> --capture --fresh-threads --models gpt-6-astra gpt-6-luna gpt-6-sol gemini-3.8-flash grok-4.7
```

The capture server is local and synthetic. It verifies catalog loading, streamed
turn completion, model switching, and the instructions actually sent in Responses
requests. `--models` selects the models to exercise; `--fresh-threads` checks each
initial base prompt independently. The default checks switching within one thread.
`--live <loopback-Atlas-base-url>` instead forwards those synthetic tasks
through the local bridge, consumes upstream tokens, and checks that the selected
models read, patch, and verify a temporary file. Only the test
fixture and synthetic instructions are sent, never private repository contents.
The script requires Python 3.12 or newer.

Both installed clients preserve the thread's initial `instructions` value when
switching models, and deliver the selected model's new literal prompt inside a
developer `model_switch` message. Validation checks the initial base against a
known catalog prompt and compares every request's latest applicable instruction
text, not just the first request. Literal example placeholders cannot bypass
verification. Fresh-thread captures match each selected profile directly.

| Upstream template | Characters | Words |
| --- | ---: | ---: |
| Astra | 21,420 | 3,314 |
| Luna | 18,037 | 2,790 |
| Sol | 18,992 | 2,949 |

These full upstream prompts are longer than the former Atlas-authored prompt.
No token or cost reduction is claimed. Tool definitions, other developer
instructions, history and caching still affect actual request usage.

## Regression Coverage

Rust tests enforce upstream hashes and complete-object equality, per-model/fallback
selection, capability defaults, reasoning choices, and context limits.
Python checks cover all-request comparisons, literal examples, legacy fields,
conflicting prompts, and stale model-switch messages.
A pricing matrix independently reconciles
all bundled models at below/exact/above tier boundaries, including cache reads,
cache writes, output-only requests, and zero/custom multipliers. Request line items
must reconcile with model totals, average costs, filtered summaries, provider totals,
and trends at the aggregate API's six-decimal precision.
