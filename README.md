# Copilot Bridge Atlas

A Desktop app connecting Codex → GitHub Copilot.

## Install and connect

Packages are a Windows x64 MSI and a macOS Intel DMG for macOS 13 or newer.
Published builds are on [Releases](https://github.com/Alan-s-projects/copilot-bridge-atlas/releases);
pull-request builds are available as workflow artifacts for validation.

On macOS, drag **Copilot Bridge Atlas.app** from the DMG into **Applications**.
The Mac package is ad-hoc signed and is not Apple notarized; macOS may require
approval in **System Settings → Privacy & Security** on first launch.

Atlas 6.x stores its data in `%USERPROFILE%\.copilot-bridge-atlas` on Windows
and `~/.copilot-bridge-atlas` on macOS. Patch updates preserve this data.

1. Open **Settings → Accounts** to sign in to GitHub.
2. Then **Settings → Models** to refresh your models.
3. Turn on the proxy switch (right top). The default address is `http://127.0.0.1:15722/v1`.
4. Open **Connect**, review the proposed TOML, copy it, and apply it yourself.
5. Reload Codex so it loads the selected provider and generated model catalog.
