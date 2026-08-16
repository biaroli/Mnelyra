<p align="center">
  <img src="static/favicon.png" width="108" alt="Mnelyra icon">
</p>

<h1 align="center">Mnelyra</h1>

<h3 align="center">One local workspace and one project memory for ChatGPT, Claude, Codex, and any MCP-compatible client.</h3>

<p align="center">
  <strong>Codex is optional: remote clients can edit files, run commands, and test projects directly through MCP. When quota gets tight, an optional ChatGPT Web bridge can add your ChatGPT Web subscription as another model path.</strong>
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

Mnelyra turns a real local project directory into a workspace that multiple AI clients can share. ChatGPT, Claude, or any compatible MCP client can read and edit files, search the project, apply patches, run commands and tests, inspect Git, and view images directly from the remote chat surface. Codex can be attached when you want its harness, but it is not required to use Mnelyra.

The Workspace is also the memory boundary. Move from one upstream client to another and the project history, current focus, recent changes, and open work can stay with the project instead of being trapped in one chat window or one model provider.

![Current Mnelyra UI](static/readme/mnelyra-connections.png)

> Every README screenshot is generated from the current Mnelyra production UI in an isolated demo environment. Project names, paths, domains, Client IDs, tokens, and credentials are fictional; no developer machine data is captured.

## Why Mnelyra

| ChatGPT Web as another model path | One workspace, shared memory | Codex is optional |
| --- | --- | --- |
| Optionally connect a ChatGPT Web bridge and use your own ChatGPT Web subscription as an additional model path instead of depending on one Codex/API quota. | ChatGPT, Claude, Codex, and other MCP upstreams can work around the same Workspace while project history and recovery state stay with that Workspace. | Any MCP-compatible upstream can operate the Workspace directly. Attach the Codex harness only when you want it; Mnelyra does not require Codex to expose local tools. |

## Feature tour

### One entry point for every upstream

Upstream clients connect to Mnelyra once. OpenAI secure connection, Cloudflare, and FRP are managed from the Connections page; Workspace switching happens behind that connection, so changing projects does not mean rebuilding every ChatGPT, Claude, or MCP client configuration.

### One control surface for the Workspace

Application-level settings stay in one place: local MCP service, permission ceiling, developer controls, and ChatGPT compaction policy. They apply across imported Workspaces instead of being rebuilt project by project.

![Mnelyra General settings](static/readme/mnelyra-general.png)

### Stable connection identity

OAuth, bearer token, and trusted local no-auth modes are supported. The installation-level OAuth Client ID stays stable while connection credentials can be rotated; switching projects does not require a new client identity.

![Mnelyra Authentication](static/readme/mnelyra-authentication.png)

### Direct project control without Codex

MCP already exposes file reads and edits, search, patches, commands, tests, Git, images, and history tools rooted at the active Workspace. Codex is an optional harness, not a prerequisite for Mnelyra.

## How it fits together

![Mnelyra architecture](static/readme/mnelyra-architecture.svg)

The client connection and project root are separate. Import projects once, then switch the active Workspace from the sidebar while clients keep using the same Mnelyra connection.

## Quick start

### 1. Install

Download the current build from [GitHub Releases](https://github.com/biaroli/Mnelyra/releases/latest).

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
| OpenAI / ChatGPT secure MCP path | OpenAI secure connection |
| Stable public hostname | Cloudflare Named Tunnel |
| Temporary testing | Cloudflare Quick Tunnel |
| Self-hosted reverse proxy | FRP |

The local MCP endpoint is typically:

```text
http://127.0.0.1:28766/mcp
```

### 4. Configure authentication and connect a client

Open **Settings → Authentication**. OAuth is recommended for a public MCP endpoint; bearer token is also available. Trusted local-only use can run without authentication.

Enter the `/mcp` URL shown by Mnelyra into ChatGPT, Claude, or another client that supports custom MCP servers. A first connection check can call:

```text
server_info
get_default_cwd
git_status
```

`get_default_cwd` should resolve to the active Workspace and `git_status` should report the same project.

## ChatGPT Web path

Mnelyra can attach Codex as an optional provider instead of making Codex a prerequisite for the whole system. With the ChatGPT Web bridge, a ChatGPT Web subscription can become an additional model path on top of the same Workspace, MCP tools, and project memory.

This is useful when a single Codex/API quota is constrained, but it still uses your own ChatGPT account and the capabilities available to that account. Browser integration can also be affected by changes to the ChatGPT web UI.

## Workspace memory

Mnelyra memory follows the Workspace. Different upstream clients can continue work on the same project without treating one ChatGPT conversation, Claude conversation, or Codex task as the only source of project continuity.

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

On Windows, the optional Codex integration keeps its workspace sandbox and scoped MiKTeX compatibility setup. This is not unrestricted host-filesystem access.

## Public routing

### Cloudflare

Named Tunnel is the long-lived option; Quick Tunnel is for temporary testing. A fixed endpoint can look like:

```text
https://mcp.example.com/mcp
```

### FRP

If you operate an FRPS server, enter its server, port, subdomain, and token on the Connections page. Mnelyra maintains one application-level FRP route rather than a profile library per Workspace.

### OpenAI secure connection

The Connections page can store a Tunnel ID and OpenAI API Key and start the secure MCP path used by the OpenAI platform. This path is independent from normal Cloudflare/FRP public routing.

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

Mnelyra was previously released as RootRelay and Codex-Web.

Thanks to [miuuyy/codex-chatgpt-web](https://github.com/miuuyy/codex-chatgpt-web), [mybolide/coding-tools-mcp](https://github.com/mybolide/coding-tools-mcp), [xyTom/coding-tools-mcp](https://github.com/xyTom/coding-tools-mcp), [Tauri](https://github.com/tauri-apps/tauri), [Svelte](https://github.com/sveltejs/svelte), and their contributors. Copyright 2026 Coding Tools MCP Contributors.

Mnelyra is not affiliated with or endorsed by OpenAI, Anthropic, or Cloudflare.
