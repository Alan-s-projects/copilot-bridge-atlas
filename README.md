# Copilot Bridge Atlas

A Desktop app connecting Codex → GitHub Copilot.

## Install and connect

Packages are a Windows x64 MSI and a macOS Intel DMG for macOS 13 or newer.
Published builds are on [Releases](https://github.com/Alan-s-projects/copilot-bridge-atlas/releases);
pull-request builds are available as workflow artifacts for validation.

On macOS, drag **Copilot Bridge Atlas.app** from the DMG into **Applications**.
The Mac package is ad-hoc signed and is not Apple notarized; macOS may require
approval in **System Settings → Privacy & Security** on first launch.

Atlas stores its data in `%USERPROFILE%\.copilot-bridge-atlas` on Windows
and `~/.copilot-bridge-atlas` on macOS. Patch updates preserve this data.

1. Open **Settings → Accounts** to sign in to GitHub.
2. Then **Settings → Models** to refresh your models.
3. Turn on the proxy switch (right top). The default address is `http://127.0.0.1:15722/v1`.
4. Open **Connect**, review the proposed TOML, copy it, and apply it yourself.
5. Reload Codex so it loads the selected provider, model list, and context settings.

Connect defaults to **Atlas customized list (Settings → Models)**. Selecting
**Codex default list** omits `model_catalog_json` and proposes
`model_context_window = 872000` with `model_auto_compact_token_limit = 780000`.
This keeps Codex's native list and instructions while requesting a larger context
budget. The installed Codex client and upstream model still enforce their limits.
The choice changes the preview only; review and apply the TOML manually. Atlas
continues refreshing its generated catalog in either mode.

## Claude Desktop and Claude Code

Atlas 7 adds a separate Anthropic Messages listener. In Settings → Routing,
configure **Responses API** (default `127.0.0.1:15722`) and **Messages API**
(default `127.0.0.1:15723`). The existing global proxy switch controls both.
Codex continues using `http://127.0.0.1:15722/v1`. Claude clients use
`http://127.0.0.1:15723` without an extra path prefix.

Configure Sonnet, Opus, Haiku, and Fable mappings using the existing selected
Copilot models. Other roles can inherit Sonnet; names shown by discovery are
the actual target model names. Connect includes setup values for both Claude
Desktop and Claude Code. See [Claude gateway details](docs/claude-gateway.md).
