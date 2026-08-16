# Mnelyra development

This repository contains one desktop application. The SvelteKit frontend lives in `src/`; the Tauri/Rust runtime lives in `src-tauri/`.

## Local development

Install JavaScript dependencies once:

```bash
npm ci
```

Run the desktop application:

```bash
npm run desktop
```

Run the frontend only:

```bash
npm run dev
```

Codex-backed features require a working Codex CLI. Mnelyra discovers `codex` from `PATH`; `MNELYRA_CODEX_BIN` can point to an exact executable/script when discovery is not appropriate.

## Required checks

Frontend:

```bash
npm run check
npm run build
```

Rust:

```bash
cd src-tauri
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Repository hygiene:

```bash
git diff --check
git status --short
```

Do not commit generated trees such as `node_modules`, `.svelte-kit`, `build`, or `src-tauri/target`.

## Runtime model

Mnelyra has one authoritative Workspace at a time. Workspace switching is not a cosmetic selection: the active local MCP runtime is drained and rebound so new requests resolve against the selected project directory.

The main runtime layers are:

```text
MCP client / ChatGPT App / other upstream
                |
       remote connection layer
 OpenAI Secure Tunnel / Cloudflare / FRP
                |
           local MCP listener
                |
          shared tool dispatcher
                |
             Workspace

Native Codex app-server is managed beside this path and is bound to the
same active Workspace and application permission setting.
```

The desktop UI intentionally does not contain a second Codex task/session console. Remote Codex-control tools and the native session coordinator remain runtime capabilities, but the user’s chat/task UI stays in the upstream client/Codex surface.

## Permission ceiling

The application setting `general.permissionCeiling` has three supported values:

- `automatic` — do not add a Mnelyra permission profile;
- `read_only` — block mutating MCP tools and mutating Codex controls;
- `custom` — Codex uses `workspace-write`, network is enabled, and Windows uses the elevated sandbox. A scoped MiKTeX writable root/environment is added only when a local MiKTeX installation is detected.

The setting is persisted globally and is applied when the MCP runtime starts. Changing it uses the `set_permission_ceiling` command, which reconfigures the native Codex app-server and restarts the running MCP service so the new policy takes effect immediately.

Read-only is enforced twice: mutating tools are omitted from the advertised tool catalog where possible, and the shared dispatcher rejects them again before execution.

## Authentication

Authentication is application-level. Workspace copies of old auth fields exist only for compatibility and are overridden at runtime by the global configuration.

OAuth uses:

- a stable installation Client ID;
- a rotatable client/connection secret;
- PKCE (`S256`) authorization codes;
- an internal token-signing secret.

There is no authorization-password page or POST authorization step. `/oauth/authorize` validates the request and PKCE parameters and redirects with a short-lived authorization code. Internal token-signing material is not exposed through frontend secret commands.

New installations generate Client IDs with the `mnelyra-client-` prefix. Existing persisted IDs are not forcibly renamed.

## Durable history and harness state

Conversation persistence is Workspace-owned. `history_session_bootstrap`, `history_session_checkpoint`, `history_session_search`, and `history_session_read` maintain a lossless numbered Markdown archive and bounded retrieval flow.

The history layer and the harness are not the runtime authority. The harness stores task/checkpoint/operation evidence; the active Workspace/runtime layer decides where live operations execute.

## Remote connections

Local MCP listeners bind to loopback. A remote client therefore needs a remote route. Mnelyra currently supports:

- OpenAI Secure MCP Tunnel;
- Cloudflare routing;
- FRP.

The OpenAI tunnel helper is an implementation dependency of Start, not a separate setup step in the UI.

Connection status shown in the topology should reflect real runtime state. Do not animate the upstream-to-computer flow merely because the local MCP listener is running; a remote route must also be ready.

## Secrets and local state

Never put runtime credentials in source, test fixtures, screenshots, or documentation. The repository should contain only secret **field names**, not real values.

Application data is resolved outside the source tree. Tests and documentation capture must use synthetic credentials and isolated data directories.

Before publishing, scan for at least:

- API keys/bearer tokens/private keys;
- OAuth client secrets;
- Cloudflare/FRP credentials;
- real Tunnel IDs;
- absolute personal project paths;
- local runtime profiles or screenshots that were not created specifically for public documentation.

## Public documentation screenshots

README screenshots live under `static/readme/` and must be captured from the current production UI with a synthetic/demo backend. Never capture a normal developer instance: Authentication, Connections, Workspace, and Memory can expose local identifiers, project paths, endpoints, and credentials.

The public images use fictional workspaces, paths, domains, Client IDs, tokens, and provider state. Temporary demo shims and browser profiles used to create those captures must be removed after the screenshots are produced.

## Naming

The product name is **Mnelyra**. Internal historical Rust crate/library identifiers may still contain `rootrelay`; changing an internal identifier is not required unless it leaks into user-visible UI, documentation, generated Client IDs, package metadata, or release artifacts.
