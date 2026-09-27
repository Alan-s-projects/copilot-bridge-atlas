# Atlas Codex Catalog

Atlas generates its own Codex catalog without reading `models_cache.json`,
looking up GPT-5.5, or borrowing another model's instructions and capability flags.

## Sources

- `src-tauri/src/resources/codex-model-template.json` contains the small,
  model-neutral Codex compatibility profile.
- `src-tauri/src/resources/codex-agent-instructions.md` is the sole instruction
  source: 564 words, applicable to every enabled model.
- Model identity, context limits, parallel calls, image input, and supported
  reasoning efforts come from the Copilot capabilities stored by catalog refresh.
  The optional existing Ultra mapping remains unchanged.

The generator emits both `base_instructions` and
`model_messages.instructions_template` from that same Markdown source. The latter
has empty personality variables; there are no separate personality prompts.

Atlas explicitly selects shell-command and freeform-patch compatibility. Tool
output truncation and the effective context budget are client-side policies.
Reasoning summaries, verbosity control, original-detail images, and native search
are not advertised based on another model's flags. Missing image and parallel-call
declarations stay conservative. Missing reasoning declarations expose `none`.
The existing 128K context fallback applies only when neither model metadata nor
an explicit context value is available.

This changes Atlas's generated catalog only. It neither modifies global Codex
configuration nor adds another settings switch.

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
```

The capture server is local and synthetic. It verifies catalog loading, streamed
turn completion, model switching, and the instructions actually sent in Responses
requests. `--live <local-Atlas-base-url>` instead forwards those synthetic tasks
through the local bridge, consumes upstream tokens, and checks that Astra, Luna,
and Sol read, patch, and verify a temporary file in one session. Only the test
fixture and synthetic instructions are sent, never private repository contents.
The script requires Python 3.12 or newer.

Measured effective instruction text changed from 19,754 characters / 3,259 words
to 3,960 characters / 564 words. This is **not a token-savings estimate**. Tool
schemas, developer context, user input, history, and caching also affect the
effective request; the capture report measures those surfaces separately.

The first matched live test captured these character counts:

| Surface | Previous catalog | New catalog |
| --- | ---: | ---: |
| Effective instructions | 19,754 | 3,960 |
| Serialized input/history | 5,155 | 5,155 |
| Serialized tool definitions | 10,620 | 19,420 |

The conservative tool profile exposes more tool definitions directly, so prompt
reduction is smaller than the instruction-text reduction alone. Before and after
the replacement, all three models completed the same read/patch/verify task with
four Responses requests per model, switching models in the same session.
These are functional smoke checks, not a token-cost benchmark.

## Regression Coverage

Rust tests enforce the single instruction source, length range, capability defaults,
reasoning choices, and context limits. A pricing matrix independently reconciles
all bundled models at below/exact/above tier boundaries, including cache reads,
cache writes, output-only requests, and zero/custom multipliers. Request line items
must reconcile with model totals, average costs, filtered summaries, provider totals,
and trends at the aggregate API's six-decimal precision.
