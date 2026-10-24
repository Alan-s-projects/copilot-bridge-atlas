# Client identification

Atlas records self-reported caller identity before replacing upstream headers.
A recognized `originator` takes precedence over User-Agent detection. Unrecognized
originators fall through. Generic SDK headers do not identify an application.
This metadata only affects request reporting, not authentication or routing.

## Claude clients

Claude Code and Desktop share the `claude-cli/<version>` User-Agent family.
Atlas reads the entrypoint in `(external, <entrypoint>[, agent-sdk/<version>])`:

| Entrypoint | Client label |
| --- | --- |
| `cli`, `mcp`, `bench`, `sdk-cli`, `sdk-ts`, `sdk-py`, `claude-vscode`, `claude-code-github-action` | Claude Code |
| `claude-desktop`, `claude-desktop-3p`, `remote_desktop` | Claude Desktop |
| Missing, malformed, or unrecognized entrypoint | Unknown |

Matching ignores case and surrounding whitespace. Versions come from the
`claude-cli/` product token, not `agent-sdk/`. Conflicting originator/User-Agent
identities do not borrow one another's version. `x-app: cli`,
`anthropic-version`, and generic Anthropic SDK User-Agents alone are insufficient.

The existing Client column displays the resulting labels. This does not add
Anthropic Messages API compatibility or configure Claude applications.

These rules are based on public source, not locally captured Claude traffic:

- [CLIProxyAPI entrypoint map and parser](https://github.com/router-for-me/CLIProxyAPI/blob/8289ec4c71ad914d0612c2776352b76ba3d355a6/internal/runtime/executor/helps/claude_client_detection.go).
- [Lynkr explanation of the shared Desktop/CLI User-Agent family](https://github.com/Fast-Editor/Lynkr/blob/4d3742a4dbd40d440b7151b7224b0b1f8d7f688a/documentation/claude-desktop.md#detection--gating).

Unlike routing/authentication checks in other proxies, Atlas does not inspect
prompts, beta flags, or user IDs to prove a first-party client. The labels are
best-effort reporting hints, not verified identities.
