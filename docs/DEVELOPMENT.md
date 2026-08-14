# RootRelay development

This document is for maintainers. The public README stays focused on installing, connecting, and using RootRelay.

## Project layout

| Path | Purpose |
| --- | --- |
| `src/` | SvelteKit desktop UI |
| `src-tauri/src/` | Rust backend, MCP server, auth, workspace runtime, tunnels, tools, and updater |
| `src-tauri/tests/` | Rust integration and contract tests |
| `src-tauri/icons/` | Tauri application icons |
| `static/` | Frontend assets and README preview |
| `.github/workflows/` | CI and tagged release automation |

RootRelay uses one application-level MCP configuration. Imported workspaces provide project roots; selecting a workspace changes the active root while keeping the configured service identity, authentication, and remote endpoint stable.

## Local environment

Use Node.js 20 or newer, Rust stable, and the platform dependencies required by Tauri 2.

```bash
npm install
npm run check
npm run desktop
```

`npm run desktop` starts Tauri development mode. Rust source changes rebuild and restart the backend process, so an MCP connection to the development instance can briefly disconnect during backend edits.

## Verification

Frontend checks:

```bash
npm run check
npm run build
```

Rust checks:

```bash
cd src-tauri
cargo check
cargo test --lib
cargo clippy --lib --all-targets -- -D warnings
```

The repository contains older Rust formatting that is not currently enforced as a repository-wide `cargo fmt --check` gate. New changes should still follow rustfmt output where practical.

## Persistent compatibility

The visible product and protocol identity is RootRelay. Some internal legacy identifiers remain intentionally compatible with existing installations.

The Tauri application identifier remains `io.github.biaroli.webcodex` so an installed predecessor is not treated as an unrelated application during upgrade. The runtime accepts both the current RootRelay macOS bundle shape and the previous bundle shape when reclaiming managed ports.

Application data now prefers a `rootrelay` directory. If a previous `web-codex-desktop` data directory exists and the RootRelay directory does not, startup attempts to migrate it and falls back to the existing directory if the rename cannot be completed.

New project history defaults to `.rootrelay/history-session/`. Projects that already contain `.web-codex/history-session/` continue to use that directory until they are migrated deliberately.

## Build packages

```bash
npm run desktop:build
```

Tauri reads the product name and updater configuration from `src-tauri/tauri.conf.json`.

## Release workflow

The release workflow builds Windows NSIS and universal macOS packages with `tauri-apps/tauri-action` and uploads updater metadata and signatures to the GitHub Release.

The repository Actions secret used for updater signing is:

```text
TAURI_SIGNING_PRIVATE_KEY
```

`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` is only needed when the updater private key was created with a password. An unset password secret is valid for an unencrypted key.

The private key must match the public updater key in `src-tauri/tauri.conf.json`. Never commit the private key. Local ignored key material may live under `.rootrelay/` or the legacy `.web-codex/` maintainer directory.

Before a release, keep the version aligned in these files:

```text
package.json
src-tauri/Cargo.toml
src-tauri/tauri.conf.json
```

After verification, create and push a version tag:

```bash
git tag v0.1.1
git push origin v0.1.1
```

The release must contain the platform installers, updater artifacts, signature files, and `latest.json`. The application updater endpoint is configured to read `latest.json` from the RootRelay GitHub Releases page.

## Repository rename checklist

When the GitHub repository itself is renamed to `RootRelay`, verify the `origin` remote and every release/updater URL before tagging a release.

```bash
git remote -v
git grep -n "Codex-Web"
git grep -n "codex-web"
```

Old names are acceptable only where the code explicitly documents backward compatibility.
