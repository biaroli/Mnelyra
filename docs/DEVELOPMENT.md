# Developing Mnelyra

Mnelyra is a Tauri 2 desktop application with a SvelteKit frontend and a Rust runtime. The product turns one local project directory into an active Workspace that ChatGPT, Claude, and other MCP-compatible upstreams can operate through a stable MCP connection.

This document is for maintainers. User-facing setup and product behavior belong in the root [README](../README.md).

## Repository layout

| Path | Purpose |
| --- | --- |
| `src/routes/` | Desktop pages and page-level UI |
| `src/lib/` | Shared components, stores, frontend API bindings, and types |
| `src-tauri/src/mcp/` | MCP transport and server integration |
| `src-tauri/src/tools/` | Workspace tools, schemas, policy checks, and dispatch |
| `src-tauri/src/workspace/` | Workspace import, activation, and resource handling |
| `src-tauri/src/session/` | Workspace memory and session continuity |
| `src-tauri/src/provider/` | Optional provider-side integration paths |
| `src-tauri/src/runtime/` | Local runtime supervision and process lifecycle |
| `src-tauri/src/tunnel/` | OpenAI secure connection, Cloudflare, and FRP routing |
| `src-tauri/src/auth/` | MCP authentication and OAuth flow |
| `static/readme/` | Public README screenshots and architecture artwork |
| `.github/workflows/release.yml` | Tagged Windows/macOS release pipeline |

## Local development

Install dependencies once, then start the desktop application:

```bash
npm ci
npm run desktop
```

`npm run desktop` starts Tauri and the Vite frontend together. Mnelyra owns Vite port `1421`; remote HMR uses `1422`. Keep those ports stable because the Tauri development URL is fixed to `http://localhost:1421`.

For frontend-only work:

```bash
npm run dev
```

The optional ChatGPT coding path currently uses the OpenAI coding runtime internally. Normal discovery uses the executable available on `PATH`; `MNELYRA_CODEX_BIN` can point to an exact executable when a development environment needs to override discovery. This is an implementation detail and should not appear as a separate user-facing product in the UI.

## Runtime architecture

The public architecture is documented in [`static/readme/mnelyra-architecture.svg`](../static/readme/mnelyra-architecture.svg). The important boundary is the Workspace: Mnelyra has one active Workspace at a time, and all workspace-scoped tools resolve against that project root.

MCP clients connect through the local MCP service or one of the configured remote routes. The routing layer does not own project state. Switching the active Workspace rebinds execution to the new project while the client-facing Mnelyra connection remains stable.

The optional ChatGPT Web bridge is a provider path, not a second Workspace. It can supply another model path while MCP tools, project state, and Workspace continuity remain owned by Mnelyra.

## Workspace activation

Workspace selection is an execution change, not a visual preference. A successful switch updates the active root, refreshes workspace-scoped runtime state, and ensures subsequent tool calls resolve against the selected project.

Do not add a second “start Workspace” action after selection. Selecting a Workspace is the activation action, and the sidebar item should not open a per-Workspace MCP/health/log dashboard. Shared endpoint/authentication controls belong under Authentication; runtime diagnostics belong under General.

Application-level settings such as authentication, connection routing, permission ceiling, and developer controls are shared across imported Workspaces. Project-specific state stays with the Workspace.

## MCP tools

The MCP surface is intentionally workspace-oriented. File reads and edits, search, patches, command execution, tests, Git operations, image inspection, and history tools all resolve through the same active root and the same policy layer.

Tool exposure and tool execution are separate checks. When a mode should hide a mutating capability, omit it from the advertised catalog where practical, but still reject the mutation again in the dispatcher. Do not treat catalog filtering as a security boundary by itself.

## Workspace memory

Memory follows the Workspace rather than a particular upstream chat. Session bootstrap, checkpoint, search, read, and validation tools preserve project continuity across ChatGPT, Claude, and other MCP clients.

The Markdown archive is the durable history. Derived state such as current focus, recent changes, open work, and provider checkpoints exists to make recovery fast; it must never replace or silently rewrite the durable history source.

Empty derived state should remain empty. Do not invent placeholder checkpoints, synthetic counts, or explanatory records just to give the UI something to render.

## Permission ceiling

The application-level permission ceiling has three user-facing modes:

| Mode | Runtime meaning |
| --- | --- |
| **Automatic** | Mnelyra adds no extra restriction beyond the active downstream policy |
| **Read only** | Mutating tools and mutating coding operations are blocked |
| **Workspace read/write** | Workspace reads and writes are allowed; the coding runtime keeps its workspace sandbox and configured network behavior |

On Windows, the workspace read/write mode keeps the elevated sandbox integration used by the coding runtime. MiKTeX compatibility is scoped to a detected local MiKTeX installation and must not turn into unrestricted filesystem access.

Changing the permission ceiling must affect the running system, not only persisted settings. Runtime reconfiguration and MCP service state must be refreshed so the new ceiling is enforced immediately.

## Authentication

Authentication is installation-level and remains stable while Workspaces change. OAuth uses one persistent installation Client ID, PKCE authorization codes, refresh tokens for long-lived clients, a rotatable connection secret, and internal signing material that is never exposed through the frontend.

Bearer-token authentication is available when OAuth is unnecessary. OAuth and bearer token are the only user-configurable MCP authentication modes.

New Client IDs use the `mnelyra-client-` prefix. Do not introduce a second authentication model for a specific Workspace or routing provider.

## Connections

The local MCP service binds to loopback. Remote access is provided through OpenAI secure connection, Cloudflare, or FRP. These are transport choices around the same MCP service; they are not separate tool runtimes.

The OpenAI secure connection uses the Tunnel ID and OpenAI API Key configured on the Connections page. Its tunnel client authenticates to the private loopback MCP listener with Mnelyra's internal tunnel token, so the OpenAI tunnel path must not add a second Mnelyra OAuth login. Cloudflare and FRP remain independent public-routing choices. Connection status in the UI must reflect actual route readiness rather than merely the existence of a local listener.

Do not duplicate routing configuration per Workspace. Routing belongs to the application connection layer.

## Frontend conventions

The UI should expose state and controls, not narrate the implementation. If the meaning is obvious from the control, prefer the control over an explanatory paragraph. Empty sections should disappear instead of rendering `0`, `0/0`, dashes, or large empty cards.

Product terminology is **Mnelyra**, **ChatGPT**, **Workspace**, **MCP**, **Connections**, and **Workspace memory**. Internal runtime names should stay internal unless a maintainer genuinely needs them to debug an implementation boundary.

README screenshots must be captured from the current production UI with isolated synthetic data. Never capture a normal developer instance because workspace paths, endpoints, Client IDs, Tunnel IDs, and credentials may be visible. Temporary demo backends, browser profiles, and capture builds must be removed after the images are produced.

## Required checks

Run the frontend checks from the repository root:

```bash
npm run check
npm run build
```

Run the Rust checks from `src-tauri/`:

```bash
cargo fmt -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Finish with repository hygiene checks:

```bash
git diff --check
git status --short
```

Generated directories such as `node_modules`, `.svelte-kit`, `build`, and `src-tauri/target` must not be committed.

## Release process

Release versions must match in `package.json`, `src-tauri/Cargo.toml`, and `src-tauri/tauri.conf.json`. The current GitHub workflow builds Windows NSIS and macOS universal packages, uploads updater artifacts, and publishes a GitHub Release when a `v*` tag is pushed.

OTA compatibility depends on the updater public key embedded in `tauri.conf.json` matching the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Keep `bundle.createUpdaterArtifacts` enabled and keep `uploadUpdaterJson`, `uploadUpdaterSignatures`, and `updaterJsonPreferNsis` enabled in the release workflow. Do not rotate the updater key casually: already-installed clients trust the embedded public key.

Release clients check the GitHub `latest.json` endpoint at startup. When a newer signed build exists, Mnelyra offers to download and install it in-app and relaunch after installation.

Before tagging, run the full checks above and verify the README images, release links, updater endpoint, and public documentation against the current UI. Then commit the release state, push `main`, create the version tag, and push the tag.

The release workflow is the source of truth for whether a release completed. A pushed tag is not a successful release until the Windows and macOS jobs finish and the GitHub Release assets are present.

## Secrets and public artifacts

Source, tests, screenshots, and documentation must never contain real API keys, bearer tokens, OAuth secrets, Cloudflare or FRP credentials, Tunnel IDs, personal project paths, or captured local profiles.

Documentation examples should use obvious synthetic values such as `example.com`, demo project names, and non-secret placeholders. Public screenshots belong under `static/readme/`; temporary capture files do not.
