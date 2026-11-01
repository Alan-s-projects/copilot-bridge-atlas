# Third-Party Notices

Atlas includes instruction data from OpenAI Codex, licensed under Apache-2.0:

- Repository: https://github.com/openai/codex
- Release: `rust-v0.163.0`
- Commit: `83086188ae0614bd882818ff1aa23f3194962012`
- Source: `codex-rs/models-manager/models.json`
- Extracted entries: `gpt-6-astra`, `gpt-6-luna`, `gpt-6-sol`, `gpt-6.1-sol`
- Snapshot: `src-tauri/src/resources/openai-codex-instructions.json`

Modification notice: Atlas extracts each entry's complete `model_messages`
object into a separate JSON snapshot. Instruction strings, nested values,
nulls, examples, and trailing newlines remain unchanged. Source and instruction
hashes are recorded in that snapshot. No OpenAI capability flags are copied.

Atlas selects each recognized GPT-6 model's own instruction object and uses
the Astra object for all other model IDs, including non-GPT models, as requested.
This prompt reuse does not change a model's actual identity, provider, or routing.

`BUNDLED_LICENSES.txt` contains Atlas's license and the complete OpenAI Codex
license and NOTICE. The Windows installer distributes it as `LICENSE.rtf`.
The attribution documents are not injected into model instructions.

Regenerate or verify the extraction with `scripts/vendor-openai-instructions.py`.

Atlas 8.1 adds the following WebSocket transport dependencies; their MIT license
texts are included in `BUNDLED_LICENSES.txt`:

- `tokio-tungstenite` 0.24.0 — https://github.com/snapview/tokio-tungstenite
- `tungstenite` 0.24.0 — https://github.com/snapview/tungstenite-rs
- `data-encoding` 2.11.1 — https://github.com/ia0/data-encoding
- `sha1` 0.10.7 — https://github.com/RustCrypto/hashes
