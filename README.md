<p align="center">
  <img src="static/favicon.png" width="108" alt="Mnelyra icon">
</p>

<h1 align="center">Mnelyra</h1>

<h3 align="center">One local workspace for ChatGPT, Codex, and Claude Code, with ChatGPT Web models available inside Codex.</h3>

<p align="center">
  <strong>ChatGPT can call local file, shell, Git, and test tools through MCP. Codex can also select ChatGPT Web models. Mnelyra manages connections, recovery, workspace memory, and context settings.</strong>
</p>

<p align="center">
  <a href="README.md">English</a>&nbsp;&nbsp;<a href="README.zh-CN.md">简体中文</a>
</p>

<p align="center">
  <a href="https://github.com/biaroli/Mnelyra/releases/latest"><img src="https://img.shields.io/github/v/release/biaroli/Mnelyra?label=release" alt="release"></a>
  <img src="https://img.shields.io/badge/Windows-x64-0078D4?logo=windows" alt="Windows x64">
  <img src="https://img.shields.io/badge/macOS-universal-000000?logo=apple" alt="macOS universal">
  <img src="https://img.shields.io/badge/license-Apache--2.0-blue" alt="Apache-2.0">
</p>

Mnelyra manages two connection directions. ChatGPT, Claude, and other MCP clients can enter the active Workspace through a remote MCP endpoint and read or edit files, search the project, apply patches, run commands and tests, inspect Git, and view images. Native Codex can use a local Responses bridge to reach the ChatGPT Web models available to the signed-in account.

Both paths share the same local project boundary. The Workspace is also the memory boundary, so project history, current focus, recent changes, and open work stay with the project when the client changes.

![Mnelyra General settings](static/readme/mnelyra-general.png)

> Every README screenshot is generated from the current Mnelyra production UI in an isolated demo environment. Project names, paths, domains, Client IDs, tokens, and credentials are fictional; no developer machine data is captured.

## What Mnelyra handles

Clients only need one Mnelyra connection. Cloudflare or FRP can expose the active local MCP service to a remote client. Switching Workspace changes the project root without forcing ChatGPT or Claude onto a new URL or a new client configuration.

ChatGPT Web can act as a local project client through a custom MCP app. Once connected, a browser conversation can call Mnelyra's file, patch, shell, test, Git, image, and history tools. The active Workspace and Mnelyra permission settings still define the boundary.

Mnelyra can also install ChatGPT Web models into native Codex. That path includes the embedded browser, Responses proxy, and Codex route; Full mode also uses OpenAI Tunnel. Mnelyra checks the real state of each part and recovers required processes at startup or after a disconnect. When Codex is actively using the browser, Mnelyra reports an active Codex task instead of treating the busy browser as an outage.

Context controls live in Mnelyra as well. **Automatic** removes fixed context and compaction overrides so Codex uses the current model defaults and its own compaction behavior. **1M** writes a `1,000,000`-token context window and a `900,000`-token auto-compaction threshold. **Custom** lets you set both values directly, without hand-editing Codex configuration files.

## How it fits together

![Mnelyra architecture](static/readme/mnelyra-architecture.svg)

The client connection and project root are separate. Import projects once, then switch the active Workspace from the sidebar while clients keep using the same Mnelyra connection.

## Quick start

### 1. Install

Download the current build from [GitHub Releases](https://github.com/biaroli/Mnelyra/releases/latest). Release builds check GitHub Releases at startup and can verify, download, and apply signed updates in-app when a newer version is available.

| Platform | Package |
| --- | --- |
| Windows 10/11 x64 | `Mnelyra_*_x64-setup.exe` |
| macOS Intel + Apple Silicon | `Mnelyra_*_universal.dmg` |

The macOS build is not currently notarized with an Apple Developer certificate. First launch may require approval in System Settings → Privacy & Security.

### 2. Add a Workspace

Use the folder button in the sidebar and select a project root. Selecting a Workspace switches the MCP root and activates that project; there is no second start button.

### 3. Choose a connection path

Open **Connections**:

| Use case | Path |
| --- | --- |
| Stable public hostname | Cloudflare Named Tunnel |
| Temporary testing | Cloudflare Quick Tunnel |
| Self-hosted reverse proxy | FRP |
| Use ChatGPT Web models inside native Codex | Web model bridge; Full mode requires OpenAI Tunnel |

The local MCP endpoint is typically:

```text
http://127.0.0.1:28766/mcp
```

### 4. Configure authentication

Open **Settings → Authentication**. OAuth is recommended for a public MCP endpoint; bearer token is also available. For OAuth, copy the Mnelyra authorization code when the browser authorization page asks for it.

Enter the `/mcp` URL shown by Mnelyra into a client that supports custom MCP servers. A first connection check can call:

```text
server_info
get_default_cwd
git_status
```

`get_default_cwd` should resolve to the active Workspace and `git_status` should report the same project.

## Connect Mnelyra to ChatGPT Web

ChatGPT cannot connect directly to an MCP server on `127.0.0.1`. In Mnelyra, configure Cloudflare or FRP from **Connections** and copy the externally reachable `/mcp` endpoint.

Enable **Developer mode** in ChatGPT, then create a custom app and connect it to the Mnelyra `/mcp` endpoint.

Open **Apps → Create**, name the app `Mnelyra`, and paste Mnelyra's remote `/mcp` endpoint as the MCP Server URL. Choose the matching authentication method, then click **Scan Tools**. If the server uses OAuth, complete the authorization prompt and wait for tool scanning to finish. Click **Create** when the tools have loaded.

In a chat, use **+ → More** to select Mnelyra. ChatGPT can then call the tools for the active Workspace. If Mnelyra's MCP tool catalog changes later, refresh the app from **Settings → Apps** so ChatGPT reads the updated tools.

![Mnelyra Authentication](static/readme/mnelyra-authentication.png)

OpenAI's current setup notes are in [Developer mode and MCP apps in ChatGPT](https://help.openai.com/en/articles/12584461-developer-mode-and-mcp-apps-in-chatgpt) and [Apps in ChatGPT](https://help.openai.com/en/articles/11487775-apps-in-chatgpt).

## Add ChatGPT Web models to Codex

Mnelyra can route native Codex through a local Responses bridge and add the account-eligible `ChatGPT Web — …` models to the Codex model list. Once installed, those models appear alongside native Codex models in the normal model selector rather than requiring a separate working UI.

Mnelyra manages the Codex route, embedded browser, and the `127.0.0.1:17841` Responses proxy. **Full** mode also uses OpenAI Tunnel for tool-enabled Codex work. `browser-only` does not start Tunnel.

An existing `codex-chatgpt-web` setup can be recovered or used to reinstall the route. Full setup reuses the Tunnel ID and Runtime Key. The Runtime Key is passed through a temporary file. First use requires ChatGPT browser sign-in.

Available models and effective context depend on the signed-in ChatGPT account and route. Mnelyra restores an existing path at startup and also exposes manual recovery from Connections. Readiness comes from the live route/doctor checks rather than the presence of a single process.

### Codex context and auto-compaction

In **Settings → General → Developer mode**, Mnelyra configures Codex `model_context_window` and `model_auto_compact_token_limit`. **Automatic** is the recommended default: Mnelyra clears both overrides and leaves context sizing and compaction to Codex. Version 0.2.6 also removes the old Web bridge's fixed `140K / 125K` limits for High/Medium. Use **1M** when you want the explicit `1,000,000 / 900,000` profile, or **Custom** when you want to set both values yourself. The active model and route still determine the effective ceiling.

## Workspace memory

Mnelyra memory follows the Workspace. Different upstream clients can continue work on the same project with the same project memory.

The Memory page shows current focus, recent changes, open work, and recoverable provider checkpoints. Durable history archives and the history tools below provide cross-client continuity.

The public history tools are:

| Tool | Purpose |
| --- | --- |
| `history_session_bootstrap` | Initialize or restore project history for a conversation |
| `history_session_checkpoint` | Save decisions, changes, verification, and next actions |
| `history_session_search` | Search previous sessions |
| `history_session_read` | Read an archived session |
| `history_session_validate` | Validate archive numbering and indexes |

Provider checkpoints are shown when the provider actually exposes recoverable checkpoint state; Mnelyra does not fabricate empty checkpoint placeholders.

## Permissions

Developer mode exposes one application-level permission ceiling:

| Mode | Behavior |
| --- | --- |
| **Automatic** | Mnelyra adds no extra restriction beyond the current downstream policy |
| **Read only** | Blocks writes, patches, command execution, and other mutating operations at the Mnelyra layer |
| **Workspace read/write** | Allows reads, writes, and network access inside the active Workspace boundary |

On Windows, the OpenAI coding path keeps its workspace sandbox and scoped MiKTeX compatibility setup. This is not unrestricted host-filesystem access.

## Public routing

### Cloudflare

Named Tunnel is the long-lived option; Quick Tunnel is for temporary testing. A fixed endpoint can look like:

```text
https://mcp.example.com/mcp
```

### FRP

If you operate an FRPS server, enter its server, port, subdomain, and token on the Connections page. Mnelyra maintains one application-level FRP route for all Workspaces.

## Run from source

You need Node.js, npm, Rust stable, and the platform dependencies required by Tauri 2.

```bash
npm ci
npm run desktop
```

Checks used before shipping changes:

```bash
npm run check
npm run build

cd src-tauri
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Maintainer notes are in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Security

Mnelyra can modify files and execute commands. Protect public endpoints with authentication and only connect clients you control or explicitly trust.

Workspace boundaries, the Mnelyra permission ceiling, and downstream sandboxes are separate layers. Mnelyra should not be treated as a complete operating-system isolation container.

The ChatGPT Web bridge is an unofficial browser integration and can be affected by web UI or account-capability changes. Use your own account and follow the applicable service terms and workspace policies.

## License

Mnelyra is licensed under the [Apache License 2.0](LICENSE).

## Acknowledgements

Thanks to [miuuyy/codex-chatgpt-web](https://github.com/miuuyy/codex-chatgpt-web), [mybolide/coding-tools-mcp](https://github.com/mybolide/coding-tools-mcp), [xyTom/coding-tools-mcp](https://github.com/xyTom/coding-tools-mcp), [Tauri](https://github.com/tauri-apps/tauri), [Svelte](https://github.com/sveltejs/svelte), and their contributors. Copyright 2026 Coding Tools MCP Contributors.

Mnelyra is not affiliated with or endorsed by OpenAI, Anthropic, or Cloudflare.
