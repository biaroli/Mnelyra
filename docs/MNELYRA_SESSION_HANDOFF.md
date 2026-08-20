# Mnelyra Session Handoff

Last updated: 2026-08-21

This is the continuity document for the current Mnelyra state. Read it before changing the Codex Web Models or tunnel lifecycle code.

## 1. Product boundaries

Mnelyra has two independent routes. Do not merge them again.

### Remote MCP route

- Local MCP service / workspace tools.
- Cloudflare Named/Quick Tunnel or FRP for public access.
- OAuth/Bearer authentication where configured.
- Current MCP development port commonly seen in this workspace: `28767`.

This route is for remote clients reaching the active local Workspace.

### Codex Web Models route

- Local Responses bridge: `127.0.0.1:17841/v1`.
- Managed hidden ChatGPT browser session.
- Native Codex `gpt-5.6-sol` model entry.
- No Cloudflare dependency, no MCP transport, no OpenAI Tunnel requirement, and no generic Custom-provider setup by the user.

Reasoning mapping:

```text
Codex low    -> ChatGPT Web Low / Instant
Codex medium -> ChatGPT Web Medium
Codex high   -> ChatGPT Web High
```

Normal user flow:

```text
Open Mnelyra
-> Connections -> Web model bridge -> Install in Codex
-> complete ChatGPT sign-in once if needed
-> start a NEW Codex conversation
-> choose GPT-5.6 Sol and Low / Medium / High
```

Mnelyra no longer injects a synthetic `mnelyra-web/*` model catalog in the normal product path.

## 2. Verified Codex configuration semantics

Mnelyra installs only the managed Codex base URL:

```toml
openai_base_url = "http://127.0.0.1:17841/v1"
```

Legacy `mnelyra-web/*` selections are migrated to `gpt-5.6-sol`. The user's reasoning effort is preserved.

The installed Codex Desktop app/core is discovered dynamically. On Windows the preferred source is the current OpenAI Codex AppX/MSIX package, then PATH/npm fallbacks. Never hard-code a package version or machine path.

## 3. Important Codex Desktop lifecycle finding

The provider endpoint is **thread/session static**, not app-server static.

Verified with the current Codex app-server:

- a brand-new thread reads the latest `config.toml` provider/base URL immediately;
- an already-loaded thread keeps the provider endpoint captured when that thread was created;
- `config/batchWrite(... reloadUserConfig: true)` does not rewrite the endpoint of an already-loaded thread;
- therefore enabling Web Models does **not** need a Codex Desktop restart;
- forcibly killing the Desktop app-server child is not a supported solution and must not be reintroduced.

On Windows the current CLI exposes `app-server daemon` commands but daemon lifecycle is Unix-only. There is no production Windows control socket that Mnelyra can rely on for unloading/rebinding existing Desktop threads.

## 4. Enable behavior

Enable sequence:

1. Discover Codex.
2. Create/reuse the managed hidden ChatGPT browser and verify authentication/temporary-chat composer readiness.
3. Start the local Responses bridge on `17841` in browser mode.
4. Install the managed `openai_base_url` route.
5. Do not restart or foreground Codex Desktop.

New Codex threads then use Web Models automatically. Threads that were already loaded before enable remain on their previous native provider, which is expected.

## 5. Disconnect behavior: native drain

Disconnect cannot simply restore config and immediately kill `17841`, because an already-loaded Web-routed Codex thread can still hold `127.0.0.1:17841/v1` as its session-static provider.

Current safe sequence:

1. Switch the bridge to native-drain mode.
2. Restore the previous Codex route/config so all new threads are native.
3. Destroy the hidden ChatGPT browser.
4. If no browser-routed request was ever seen, release `17841` immediately.
5. Otherwise hand `17841` to a small native-drain helper that forwards stale loaded-thread requests to the official Codex backend.
6. The drain helper watches the owning Codex Desktop app-server PID and exits when that owner exits. A later Web Models enable or explicit internal stop also removes the drain before browser mode starts.

This prevents both failure modes: no stale loaded thread is stranded, and ChatGPT Web is no longer consumed after disconnect.

The native-drain helper was tested both with the real Desktop app-server and with a controlled short-lived `codex.exe` owner. Owner exit released the helper and port automatically.

## 6. Foreground safety

Do not use Windows UI automation to drive Codex Desktop.

Real experiments proved that all of these can steal foreground focus:

- `SetForegroundWindow`;
- `SendInput` / `SendKeys`;
- UI Automation `SetFocus()`;
- UI Automation `Invoke()`.

The user explicitly rejected foreground stealing. Product and test flows must use the hidden Mnelyra WebView, Codex core/app-server, local Responses bridge, and logs instead.

The real Desktop High proof was obtained before this rule was locked down; no further foreground automation is necessary.

## 7. Real Web Models validation

### Real Codex Desktop High

A real Codex Desktop `gpt-5.6-sol` High turn was isolated in `~/.codex/logs_2.sqlite` and proved:

- request target `http://127.0.0.1:17841/v1/responses`;
- HTTP `200 OK`;
- `item/agentMessage/delta` events;
- `turn/completed`;
- visible Desktop response marker `WEB_OK_A`.

This proves the actual Desktop -> Mnelyra -> ChatGPT Web route, not only `codex exec`.

### Streaming

A long structured Markdown direct bridge run emitted 56 deltas before completion. A Codex E2E run emitted 69 deltas before completion. Final Markdown and terminal markers were preserved.

Do not regress to completion-only buffering or common-prefix abort behavior. The active unstable tail remains buffered while stable final-answer prefixes stream incrementally.

### Low / Medium / High

All three reasoning tiers passed real hidden-browser + Responses bridge + Codex E2E tests on 2026-08-21:

```text
Low:    PASS
Medium: PASS
High:   PASS
```

The Low run exposed a real slider-path JavaScript bug (`options` out of scope in diagnostics). It was fixed and Low passed on rerun. Keep explicit debug flags available:

```text
--web-models-selftest-low
--web-models-selftest-medium
--web-models-selftest-high
--web-models-selftest-stream
--web-models-selftest-hold
```

Self-tests must remain hidden. If authentication is missing, they fail instead of opening/focusing a window.

## 8. Request/stream behavior that must remain stable

- `/v1/models`: native passthrough.
- `/v1/responses`: browser bridge for native Sol/legacy Web requests while browser mode is active.
- `/v1/responses` in native-drain mode: official Codex backend passthrough.
- `/v1/responses/compact`: native Sol passthrough to official Codex backend; do not invent Web compaction semantics.
- other supported Codex endpoints: native passthrough.
- websocket GET on local Responses is not the production transport; HTTP Responses streaming is the supported bridge path.
- compressed Codex request bodies are decoded safely for browser routing.
- prompt chunking must preserve Unicode exactly and remain fast for large contexts.

## 9. Context policy

Do not change context/summary semantics while touching transport.

Current controls:

- Automatic: clear explicit Codex context/auto-compact overrides.
- 1M preset: `1,000,000` context and `900,000` auto-compact threshold.
- Custom: explicit user values subject to validation.

The one-million context policy tests pass in the current full suite.

## 10. Cloudflare / RootRelay boundary

RootRelay is only the development connector used to edit Mnelyra during this work. Do not treat its runtime as Mnelyra Web Models state and do not copy its tunnel state into the model route.

Mnelyra tunnel lifecycle hardening in the current worktree includes ownership-aware process cleanup and fast failure when the local MCP port is unavailable. Do not blindly terminate unrelated `cloudflared.exe` processes.

RootRelay source remains a separate later task and must not be edited from this Mnelyra workspace.

## 11. Current test baseline

Release-candidate checks on 2026-08-21:

```text
Web Models focused Rust tests: 18 passed, 0 failed
Full Rust library tests:       154 passed, 0 failed
Svelte check:                  0 errors, 0 warnings
Frontend production build:    PASS
Clippy -D warnings:            PASS
Real Web Low E2E:              PASS
Real Web Medium E2E:           PASS
Real Web High E2E:             PASS
Long incremental streaming:    PASS
Real Desktop High route:       PASS
```

The full test count is 154 because an obsolete bundled-catalog parser test was removed together with the now-unused synthetic-catalog code.

## 12. Clean runtime expectations

After self-test/release verification and with Web Models disconnected:

```text
Codex model:             gpt-5.6-sol
reasoning effort:        user-controlled (latest clean check was max while Web Models was disconnected)
managed openai_base_url: absent / restored
hidden browser:          absent
```

Port `17841` should be down when no stale loaded Web thread needs draining. A production disconnect may intentionally keep only the native-drain helper alive while its owning Desktop app-server remains alive; that helper performs no ChatGPT Web inference and exits with its owner or on the next enable.

Latest observed Codex package during validation was `OpenAI.Codex 26.818.2872.0`. This is diagnostic history only; never encode it into product code.

## 13. Release / repository rules

- Product code must be portable: no developer-machine absolute paths or credentials.
- Do not commit `.rootrelay` diagnostics, extracted Codex bundles, local history directories, generated build caches, or test binaries.
- README should stay user-facing: functionality and setup steps only, not internal reverse-engineering notes.
- `docs/MNELYRA_SESSION_HANDOFF.md` is the engineering continuity document and may contain the technical rationale above.
- The user explicitly requested that this completed work be committed, pushed, and released after verification.

## 14. Definition of done

For this Web Models phase, done means:

1. Actual Codex installation is discovered automatically.
2. User never configures a generic Custom provider.
3. Native `gpt-5.6-sol` maps Low/Medium/High to the corresponding ChatGPT Web modes.
4. New Desktop threads pick up enable/disable route changes without Desktop restart.
5. Already-loaded Web threads survive disconnect through native drain instead of breaking or continuing Web inference.
6. Real Desktop High route is proven.
7. All three Web reasoning tiers pass E2E.
8. Long final answers stream incrementally through `17841`/Codex.
9. Hidden browser does not steal foreground.
10. Route/config restoration is reversible.
11. Context policy remains intact.
12. Focused/full Rust, Svelte, build, and Clippy checks pass.
13. README/tutorial reflects the native-Sol workflow.
14. Release source contains no local diagnostic/build garbage.

At the 2026-08-21 release-candidate checkpoint, items 1-13 are verified. Item 14 is the final repository/build-cache cleanup performed after the release artifacts are produced and uploaded.
