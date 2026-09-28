# Copilot Bridge Atlas

A Desktop app connecting Codex → GitHub Copilot.

## Install and connect

Download the Windows x64 MSI from [Releases](https://github.com/Alan-s-projects/copilot-bridge-atlas/releases).

Atlas 6.x stores its data in `%USERPROFILE%\.copilot-bridge-atlas`. Patch updates preserve this data.

1. Open **Settings → Accounts** to sign in to GitHub.
2. Then **Settings → Models** to refresh your models.
3. Turn on the proxy switch (right top). The default address is `http://127.0.0.1:15722/v1`.
4. Open **Connect**, review the proposed TOML, copy it, and apply it yourself.
5. Reload Codex so it loads the selected provider and generated model catalog.
