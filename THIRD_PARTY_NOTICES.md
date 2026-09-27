# Third-Party Notices

Atlas includes instruction data from OpenAI Codex, licensed under Apache-2.0:

- Repository: https://github.com/openai/codex
- Commit: `d8ec479c34895214b44c062f060d97224c191a50`
- Source: `codex-rs/models-manager/models.json`
- Extracted entries: `gpt-6-astra`, `gpt-6-luna`, `gpt-6-sol`
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
