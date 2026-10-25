# 2026-10 round: Vestra Player AI (Plugin Workshop rework)

Started 2026-10-24. Requirements and user rulings: [vestra-ai.md](../../product/vestra-ai.md). **This document is the only progress record for this round.**

## Author's plan (2026-10-24, reviewed by the coordinator)

- **manifest**: the id stays `vestra.workshop` (changing the id would change the installed instance and its data); the display name becomes Vestra Player AI; capabilities add `secrets`, `ui.settings`, `clipboard`.
- **Process structure**: the plugin process owns the UI, session metadata, keys, settings and privileged tools (typecheck/pack/validate/trial/install/export). Each running session gets one agent child process: `spawn(process.execPath, …)` plus IPC, running pi-coding-agent 1.4.0 (pi-ai and pi-agent-core at the same version, all pinned to exact versions).
- **Child process permissions**:
  - `--permission --allow-net`;
  - readable: the plugin package directory, this session's directory, the SDK types directory; writable: only this session's directory;
  - no child processes, workers, addons or wasi;
  - environment variables: `PI_OFFLINE=1`, `PI_TELEMETRY=0`, `PI_PACKAGE_DIR`.
  - Tested with a probe: out-of-bounds reads and writes and spawn are all denied; IPC works.
- **pi configuration**: in-memory credentials (no `auth.json` written), in-memory settings; ResourceLoader auto-discovers nothing; the system prompt and SDK docs come from the plugin.
- **Key path**: keychain → `secrets.get` → IPC → child process memory. Never through environment variables, command-line arguments or files; a test asserts that the session JSONL contains no key.
- **Tools**:
  - read/write/edit/ls/find use pi's tool definitions, but with our own file layer injected: confined to `work/`, checked with realpath; symlinks and path components starting with `.` are always rejected; each file ≤1 MB.
  - grep is implemented in pure JS (pi's own spawns rg).
  - Plugin tools (scaffold/typecheck/check/trial/install) are handed to the plugin process over IPC; install must first be confirmed in the UI, then by the host.
- **Bundling**: the full pi bundle is 14.2 MB, 7.6 MB minified; the goal is a worker ≤5 MB (drop tui/CLI/bedrock/jiti/highlight.js).
- **Session directory**: `<data>/sessions/<sid>/{meta.json, *.jsonl, work/<plugin-id>/, dist/<id>-<ver>.vxp}`.

## Progress

- [x] Vertical slice code done (2.0.0, PR awaiting merge, 2026-10-24; pi bundle 4.8 MB, vxp 5.1 MB, tests 18/18). Awaiting real-device acceptance: menu → no-key banner → save key on the settings page → new chat → real provider streaming reply (plain text) → write tool card → file appears in `work/`; including stop, error bar, new chat/switch session. No new API needed.
- [ ] Stage 2: Markdown messages (waits on R-a)
- [ ] Stage 3: plugin tools, install, export .vxp/zip (waits on R-h)
- [x] Real-device acceptance (2026-10-24, MiniMax-M3.1-Flash, "Custom · OpenAI format"): 2.0.3 passed:
  - replies, sessions, tool calls (write/read/ls), rolling status line, expand, fetch models, old settings migration;
  - when keychain authorization is refused the plugin keeps running, shows a notice bar, and cannot send;
  - the key is not written to disk, the session directory has no hidden files, and there is only one `--permission` child process.
  - Not tested yet: Enter with a Chinese input method, stop.

## API requests

- **R-a**: `MarkdownRef` can be placed in list rows; add a non-scrolling `MarkdownBlocks` component. Blocks stage 2.
  - **Maintainer reply (2026-10-24): accepted after rewrite.**
  - `MarkdownRef` may appear at any depth of a property value; the Slint type it sits in must be `[MarkdownBlock]`; add a `MarkdownBlocks` component (no scrolling, height follows content), with optional `code-copy` via a `copy-code(string)` callback.
  - R-b gets no API; the host does it internally: cache by fingerprint, do not re-parse unchanged documents, diff the block model in place, push in place when lines are appended.
  - A nested `ImageRef` explicitly reports `invalid`.
  - SDK 1.5.4, after 1.5.2 and 1.5.3.
  - **Author: accepted.**
- **R-h**: run the SDK's check/pack (`@vestra-player/sdk/tooling`) inside the installed player too. Right now `stage-runtime.mjs` does not include `bin/` or typescript. Blocks stage 3.
  - **Maintainer reply (2026-10-24): rejected, with an alternative.**
  - Every user's runtime should not carry about 20 MB extra (about 3.6 MB gzipped) when only one plugin uses it.
  - Alternative: the plugin package ships the SDK's `bin/`, `dist/`, typescript and `@types/node` itself (the current workshop already ships tsc), runs `vestra-plugin check|pack` in a child process, and then hands off to the existing `packages.validateArchive`, `trial`, `install`.
  - If structured diagnostics are really needed, add `--json` to the CLI later (SDK patch).
  - **Author: accepted the alternative.** If the problem of the SDK build skipping the vendored `node_modules` layout cannot be solved, the author will report it as a migration blocker.
- **R-c** (later): copy callback for code blocks.
- Not raised: R-b, R-d, R-e, R-g; R-f proved unnecessary.

## User rulings

- 2026-10-24 **Remove the trial feature entirely** (host, SDK, docs), with no deprecation period. Verified that only the AI plugin calls `candidate.trial()`: the store, sideloading, plugin management, the SDK examples and the plugin repository do not use it; the info and plugin management plugins only have display text for origin = trial, which is removed too. The AI plugin's flow becomes "write → typecheck → test → check → install (revertible)". R-i and R-j are void. SDK 1.5.8 removes it (patch); `apiVersion` stays `vestra/1`.

- 2026-10-24 **Coding plans and subscription providers are never put in the list** (Kimi For Coding, Z.AI/Zhipu Coding, Qwen/Xiaomi Token Plan, OpenCode Go, etc.). These providers' policies change, we cannot keep up, and users could get their accounts banned. Users who need them can connect with "Custom · OpenAI/Anthropic format" and a URL at their own risk. Revisit later. (Finding: the `User-Agent: pi/<ver>` header pi sends is unrelated to the telemetry setting, and we must never change it.)

- 2026-10-24 Providers: **no special cases for individual providers**.
  - The dropdown lists all mainstream providers pi supports (API key only); model lists come from pi's catalog;
  - plus two entries "Custom · OpenAI format" and "Custom · Anthropic format": enter a Base URL and key, fetch the model list automatically where possible, and enter the model id by hand when that fails;
  - each provider's key is stored in the keychain separately.

- 2026-10-24 Message rendering: **Markdown, not HTML+JS**. HTML would need a browser engine embedded in the host; the change and the security surface are too large. Streaming is throttled to about 100 ms and the whole message is re-parsed. Tool calls are shown as **one rolling status line**: the newest replaces the previous one in place, and after the turn ends it can be expanded to show all; it must refresh live. Streaming of reply text can be skipped for now if it is difficult.

- 2026-10-24 Source zip export: **the author's own repository plus a registry entry**. The zip is a complete plugin project (README, tests, CI for releases) that can be turned directly into the user's own GitHub repository; to publish, submit a `registry/<id>.toml` entry to the official plugin repository. The plugin repository needs a path for "third-party entry, built from source"; that is plugin repository infra work and is a separate project.

## Leftovers

- In `src/packaging/third-party.toml` the workshop entry lists only 4 packages, while 61 are actually bundled; it should point to `dependency-inventory.json` instead. Owned by packaging maintenance.
- "Retry" resends the last user message, so the transcript shows a duplicate (pi 1.4 has no regenerate).

## Stage 3 (2.2.0, branch ai/stage3, 2026-10-24)

Done:
- Knowledge directory: the SDK's .d.ts, examples and llms.txt;
- system prompt rewritten, with explicit completion criteria;
- tools: scaffold / typecheck / test (`--permission` child process) / check / pack / trial / install;
- result cards: trial, install, revert, export .vxp, export source zip, open folder;
- the author prefix is `local.` or the GitHub username;
- evaluation: 6 tasks plus a harness.

New requests from the author (awaiting maintainer reply):
- **R-i (host)**: trial only supports commands/ui.window/events; other capabilities return `unsupported`, so most generated plugins cannot be trialled. Wants trial to support all capabilities and revoke all resources on stop.
- **R-j (SDK)**: the STARTING list of `trial()` is missing the host's `"pending"`, so the state stops updating and the failure reason is lost; the author polls `packages.list()` as a workaround for now.
- **R-k (host)**: on install the host shows no confirmation of its own; there is only the plugin-side confirmation.
- **R-l (host)**: `validate` candidates are never released, capped at 32 per run; wants a way to release candidates, or a check that only compiles Slint.
- **R-m (SDK CLI, not blocking)**: under `--permission`, `existsSync` throws on paths without permission, so the CLI is run in an unrestricted child process instead.
- Also: `third-party.toml` needs the vendored SDK added.

## Field test (2026-10-24, 2.2.0, MiniMax-M3.1-Flash, author zysilm)

1. **Timer window** (`zysilm.timer`: menu + window + button)
   - About 1 minute, 24 tool calls, 0 failures, passed first time: read llms.txt, .d.ts and the widget library first, then scaffold, write code; typecheck, test and check all passed.
   - **Trial failed**: the card said "the trial did not appear in the plugin list"; cause not found, possibly related to R-j.
   - Install succeeded; the menu "Tools ▸ Timer" works and the +1 counter is correct.
2. **Remaining time OSD** (`zysilm.remaining-time`: shortcut Shift+R + ui.osd + playback.read)
   - About 2 minutes, 51 tool calls, 8 failures; in the end typecheck, test (6 cases) and check all passed.
   - Not trialled because of R-i (trial only supports commands/ui.window/events).
   - Not installed yet, and not yet used on a real device.
3. Task 3 not done.

**Observed problems**
- Replies contain English process narration ("I'll start by reading…"). The system prompt should require process narration in the user's language, or no process narration at all.
- The 8 failures have not been analysed one by one yet.
- The cause of the timer trial failure must be found.

### Maintainer replies to R-i through R-m (2026-10-24)

- **R-i, R-j: no longer apply**; trial has been removed.
  - Root cause of the trial failure in the field test: the host lists a trial plugin as `<id>.trial.<n>`, while the plugin looked it up by its original id; also the SDK's "starting" list was missing `"pending"`. The timer plugin itself was fine.
- **R-k: rejected.**
  - Reason: `plugins.manage` is a trusted capability, and each entry point confirms in its own UI; a second host dialog would duplicate the confirmation in flows such as the store.
  - Alternative: keep the plugin-side confirmation, listing id, version and capabilities.
- **R-l: fixed as a bug (player 0.4.2).** Only the latest 32 candidates are kept; installing with an old token reports `stale`. "A check that only compiles Slint": rejected.
- **R-m: fixed as a bug (SDK 1.5.8).** The CLI treats `ERR_ACCESS_DENIED` as file not found; typecheck in permission mode still needs `--allow-child-process`.
- **SDK 1.5.8 / player 0.4.2** (PR sdk#24, player#15): remove trial, R2 `resolvers.fileTypes`, R-l, R-m.

## Field test round 2, task 3, and 2.2.2 (2026-10-24, author)

Task 3 "Speed nudge" (`zysilm.speed-nudge`, 2.2.1, MiniMax-M3.1-Flash): 69 tool calls, 28 failures; typecheck, 6 tests and check passed, install succeeded, and the menu "Playback ▸ Speed nudge" is there.

**Breakdown of the 28 failures**
- **23 (ls 1, read 2, grep 20): absolute paths to reference material copied wrong.** The paths are about 150 characters, and the model dropped a middle segment (`-Users-ziyang-Develop-vestra-player` became `-Users-vestra-player`), getting the same file right once and wrong once. The cause is ours: both the prompt and the grep results give long absolute paths. Knock-on effect: testing.d.ts was never read, causing 3 later typecheck failures (`TestHost.state`, `player`, `playback` were all guesses).
- **3 typecheck, 2 test**: normal iteration; 1 of the test failures was `Shift+]` being rejected by the SDK (see below).

**Fixes in 2.2.2**
- Reference material uses short paths `sdk/…` (`sdk/llms.txt`, `sdk/dist/plugin/types.d.ts`), accepted by read / ls / find / grep, and grep results are shown the same way; writes and deletes under `sdk/` are always rejected. Absolute paths still work.
- Opening line ("I'll start by reading the SDK documentation."): the prompt already required process narration in the user's language, and the model still wrote the first sentence in English. The prompt alone is not reliable, so two layers: the prompt adds "output no text before the first tool call"; in the UI, text that comes before the first tool call of a turn and is only one line (≤200 characters) is not shown, while the transcript keeps the original. Later narration and replies without tool calls show as usual. The rule is only "position + one line", with no language detection. Cost: for a reply without tool calls, the first line only appears once the second line starts (or it passes 200 characters, or the reply ends).
- Shortcuts: the prompt states that symbol keys are written as the character actually typed (Shift+] is `}`, Shift+[ is `{`), and the key the user asks for is implemented as is. This time the model read the SDK error "write the character Shift produces instead of Shift+]" as "drop Shift", bound `]` and `[`, which conflict with A-B loop, and the player ignored them (only in the host log).
- Conflicts made visible: the public API can see them (`shadowed` in `bindings.list()`), so no request was raised. After install the plugin reads the binding table once and writes occupied keys into the card status and the install_plugin result, for example "Shortcut not active: ] (speed up 0.25×) is already used by vestra.marks "Set B point"", and has the AI suggest another key. For this the manifest adds `bindings.manage` (used read-only).

**New requests from the author (awaiting maintainer reply)**

- **R-n (SDK CLI, not blocking): `build` / `pack` fail under `--permission`.**
  - Location: `vestra-plugin build` (`build()` in `bin/vestra-plugin.mjs` copies package files with `cp` from `fs/promises`).
  - Repro: `node --permission --allow-fs-read=<project>,<SDK> --allow-fs-write=<project>/dist,<tmp> --allow-child-process vestra-plugin.mjs build <project>`, with `ui/main.slint` in the project → `ERR_ACCESS_DENIED` (FileSystemRead, resource is the project's parent directory). Cause: Node's `fs.cp` checks every parent directory of the target (checkParentPaths), and for files in subdirectories it stats all the way up to the root. `copyFile` and `cpSync` are not affected (tested).
  - Wanted: copy single files with `mkdir(dirname, { recursive: true })` + `copyFile` instead, so build / pack can also run under `--permission`.
  - Impact: implementation detail only, no contract change. Right now the workshop only runs typecheck (`check`) in permission mode; check and pack are still unrestricted.
  - Acceptance: the command above passes for both build and pack.
- **R-o (SDK wording, not blocking)**: change `binding Shift+]: write the character Shift produces instead of Shift+]` to give the character directly, for example `binding Shift+]: write "}" (the character Shift+] types)`. When the character depends on the keyboard layout, at least give the US layout example. The model took it literally as dropping Shift and bound `]`.
