# 2026-10 round: Slint upgrade and plugin store

Started 2026-10-22, baseline main 8bc172b. The user authorized the coordinator to proceed autonomously (while the user is away): checkpoints that were meant for user review are decided by the coordinator on the established principles, recorded under "Coordinator decisions" for the user to review afterwards.

**This document is the only progress record for this round.** After a context compaction the coordinator reads this document first, then continues; after each completed step this document is updated and committed.

## Goals

1. **Task 1: Slint 1.16.1 → 1.18.x**. Adapt to the new version, remove the workarounds that exist because of 1.16, and run the full checks and a full real-device regression.
2. **Task 2: plugin store (ecosystem plan S5)**. Store plugin: list, details with README, permissions, signature verification, third-party notice, revocation, update and rollback. During acceptance, also test the three items stage 4 could not test: installing a `.vxp`, replacing the same id across sources, installing from the Plugin Workshop.

Run in order: task 1 first, then task 2 (the store UI is built on the new Slint). After that come S6 overall acceptance (including a real Windows machine), S7 packaging (bundled Node 24, .app, signing, Windows installer) and third-party license collection, which are not in this round.

## Roles and process

- All code is written by sub-agents, one new agent per stage, each working in its own worktree and delivering a report in the fixed format. The coordinator does review, merging, the full checks (community-workflow §6.1), real-device testing, GitHub operations and small doc changes.
- Task 1 is top-down (§5): maintainer research → list (coordinator decides) → maintainer implements → plugin authors change their own .slint where needed → SDK release where needed (§1.6) → full real-device regression → merge.
- Task 2 is bottom-up (§4): the store plugin author writes a design and lists API gaps (coordinator decides) → if there are gaps, the maintainer goes through an API proposal → author implements → real-device acceptance → merge.

## Small issues assigned to this round

| Issue | Assigned to |
| --- | --- |
| The 512-entry action history cap is never cleaned up | Task 1 maintainer |
| The `portability` check writes files into `src/host/target/portability` | Task 1 maintainer |
| Keychain entries remain after uninstalling with data | Task 1 maintainer |
| Subtitle plugin test fails intermittently | Task 1 maintainer (locate; plugin-side changes go to the subtitle author) |
| Old `src/` commands in the plugin repository's youtube/bilibili READMEs | Task 2 (plugin repository) |
| `nes/package.json` version 0.1.0 does not match manifest 0.2.0 | Task 1, in passing |

## Known facts

- Slint 1.18.1 trial build (2026-10-22): after changing the two version pins (`src/host/Cargo.toml`, `src/host/unsafe/render/Cargo.toml`) to `=1.18.1`, production code builds with no errors and no warnings; test code `ui_markdown.rs:760,803` uses `StyledText::as_ref()`, which 1.18 no longer has, and needs changing. Plugin .slint files have not been checked under the new version.
- 1.16 workarounds (incomplete; the research must complete the list): the ComboBox `<=>` global pre-check `ui_probe.rs` (fixed upstream in 1.18); Markdown italics turned amber, strikethrough turned grey, code split into ASCII/non-ASCII fonts, hand-drawn task boxes (`ui_markdown.rs`, `host/ui/vestra/markdown.slint`).
- The plugin repository is public and the release flow works: Releases `vestra.youtube-v0.2.0`, `vestra.bilibili-v0.2.0`; `https://vestrasys.github.io/vestra-player-plugins/index.json` passes signature verification. Public key `keys/index-signing.pub.pem` (plugin repository); the signature is base64, strip leading and trailing whitespace before verifying. The version range syntax of `revoked.json` is in the plugin repository's `scripts/lib/semver.mjs`.

## Progress

- [x] 1.1 Maintainer research: changelog, workaround list, scan of all .slint files (report: [2026-10-slint-store-research.md](2026-10-slint-store-research.md); the W1–W21 numbering follows it)
- [x] 1.2 Coordinator decides the list
- [x] 1.3 Maintainer implements (including the small issues): `rebuild/slint-118` d2ae8dd
- [x] 1.4 Plugin .slint adaptation: `rebuild/slint-118-plugins` 021a967 (playlist, audio, video, visualizer version bumps; subtitles test de-flaked, 52 concurrent runs with 0 failures; nes version aligned)
- [x] 1.5 SDK 1.1.2 released; the player depends on ^1.1.2 (13a8d69)
- [x] 1.6 Full checks + full real-device regression. Merge branch `rebuild/slint-118` (scratchpad/wt-core, 4f19401): all of §6.1 passed (cargo test 294, 17 plugin tests, test:sdk 11/11, portability clean). **Real-device regression done from 2026-10-22 07:50, no upgrade regressions, see "Real-device test record".** Launch script `scratchpad/run18.sh <suffix> [--plugin …]`; regression list in research report §4.3 plus workflow §6.2; media in `scratchpad/media` (hd.mp4, hd.srt, tone.mp3, test.nes, etc.)
- [x] 1.7 Merge (together with the store, main 615bd07)
- [x] 2.1 Store plugin author design + API gaps: `doc/architecture/store-design.md` (branch `rebuild/store-ui` c504731). No blocking gaps for v1 (G1 list icons and G2 window drag-and-drop are optional; G3 is a contract clarification)
- [x] 2.2 Coordinator decides the design
- [x] 2.3 API proposal and implementation: not needed (G3 is only a doc clarification, see decisions)
- [x] 2.4 Store plugin implementation: `rebuild/store-ui` 6481427 (scratchpad/wt-plugins2), 47 tests pass, smoke test against the live index passes. Real-device driving notes and acceptance list in design §8.2. Security review (independent agent, adversarial) done: no exploitable flaws in signatures, downloads or package verification; found 1 high (revocation has no effect after delisting), 1 medium (replaying an old index hides revocations), 4 low, all fixed (34c1822); README/icon hashes done together with the plugin repository hardening (dc58f40)
- [x] 2.5 Real-device acceptance (including the three items left from stage 4): done and retested 2026-10-22, see "Real-device test record"
- [x] 2.6 Merge: main 615bd07 (Slint 1.18, store, system fonts, close shortcut, SDK 1.1.3, real-device bug fixes)

## Coordinator decisions

(Each entry states: date, decision, reason, and what is affected if the user overturns it on review.)

**2026-10-22, step 1.2 (workaround list)**
1. Follow research report §4.1, adopting its "can remove / needs rewriting / must keep" conclusions.
2. W1: remove the pre-check that changes the global on a separate thread; keep the single-page compile when an options page is registered (failure is still `ui-prepare`). A test proves the reverse: ComboBox two-way binding to a global no longer crashes.
3. **W4, the forced Chinese font, is not removed this round.** Reason: it would change Latin glyphs and sizes across the whole UI; it is a look-and-feel decision that the user should make after seeing a comparison. During regression the coordinator puts a comparison screenshot with `VESTRA_FONT_FALLBACK=system` in this document for the user.
4. W7 is rewritten as the report says (inline code as one run, code blocks in a monospace font); W8's `catch_unwind` stays (it still relies on a private interface, and keeping the guard costs very little).
5. W21: do both `close`/`hide` renames. Deduplicate glow if it can be done painlessly; otherwise keep both versions and explain in the report.
6. Layout deferred by one frame (report §1.3 risk 2): the maintainer must find and fix the places in host production code that read geometry right after switching an `if`, not just change the tests.
7. Operation receipts: evict the oldest terminal receipts as in report §5.1.
8. Keychain: as in report §5.3, the host records key names and deletes them on uninstall; cap of 256 names per instance, beyond which `secrets.set` fails. Reason: without a cap a plugin could make the host's index grow without bound; 256 is far above normal plugin usage. Reuse existing error codes where possible, add none. The user can overturn this (impact: one reference rule and one test).
9. Release SDK 1.1.2 (markdown.slint sync, `testing.ts` default version, `settle()` docs). Do not add `host.until`.
10. Order: the maintainer does host + SDK first (1.3/1.5); after that is merged, plugin authors change their own .slint on 1.18 (1.4, including playlist, audio, video, subtitles test de-flaking, nes version alignment), so plugins do not use new property names on a 1.16 host.

**2026-10-22, step 1.3 review**
1. Accept `ui::settle_layout` calling the private interface `WindowInner::ensure_tree_instantiated` (instantiate `if`/`for` before reading geometry). Reason: there is no public alternative, and both `show()` and dispatching events have side effects; the Slint version is pinned exactly, and an upgrade would be caught by the test `geometry_is_current_after_a_condition_changes`. Recorded in the architecture README.
2. The binding loop warning on `sidebar-actual` in the main window's generated source (present since 1.16) is not fixed this round and goes to the to-do list: fixing it changes the sidebar layout semantics and needs its own round with real-device verification.
3. With the mount pre-check removed, a panel that "compiles in the main window but fails to compile as a separate floating window" only fails when detached. Accepted: the failure only affects that floating window, and every panel has already compiled in the main window.
4. SDK 1.1.2 is merged, tagged and pushed by the coordinator; CI publishes it.

**2026-10-22, step 2.2 (store design)**
1. Adopt the design. The store is a separate built-in plugin `vestra.store`, apart from plugin management (reasons in the design: the networked part can be disabled on its own, failures do not drag each other down, the layouts differ).
2. The entry goes at the top level as "Plugin Store…", right next to "Plugin Workshop…" (the workshop is already top level; similar entries sit together). The title shows a count when there are updates. The user can overturn this and move it under "Settings ▶"; the impact is just one menu order.
3. The store offers uninstall and rollback (like VS Code); plugin management keeps its existing buttons; both call the same API, so there is no conflict.
4. Revoked plugins: disabled automatically once, with a persistent warning; if the user re-enables manually they must first confirm the risk, and after that it is not force-disabled again.
5. Third-party repositories in v1 are only marked "unofficial" with a warning: no public key configuration, no TOFU, and README does not load remote images by default.
6. Automatic check: once 30 seconds after activation, then every 24 hours; on by default, can be turned off on the settings page.
7. G1 and G2 are not done this round (no icons in the list; drag onto the video area to install + an "Install from File…" button). G3: installing a lower version from the same source is allowed, as the host does today; the maintainer adds a sentence to reference §12 (done together with 2.4).
8. Plugin repository index adds a monotonic `generated` field and signatures support multiple signatures: goes to the to-do list, does not block S5.

**2026-10-22, user rulings (after real-device testing)**
1. Fonts (W4): must support multiple platforms. Principle: use an additionally installed font if it is better, otherwise the system default. The coordinator judged the system default: 1.18 can already fall back per character to show Chinese, and system fonts (PingFang on macOS, YaHei on Windows, the Noto CJK that Linux distributions ship, the system fonts on Android/iOS) are closest to native on each platform, add no dozen-plus MB to the installer, and add no extra font license; a bundled font would not solve the italics problem either. So the forced font is removed in favour of the system default + Slint fallback. S6 verifies this on a real Windows machine; if fallback fails on some platform (for example a minimal Linux without a Chinese font), add a fallback font (Noto Sans SC, OFL) only for that platform and leave the others unchanged.
2. Shortcut to close tool windows: support it, with attention to multiple platforms. Approach: the host provides a "standard window shortcuts" table in the platform layer, and tool windows and floating panels close by platform convention: macOS ⌘W; Windows Ctrl+W and Ctrl+F4; Linux Ctrl+W. The main window does not respond (single-window player; closing the main window equals quitting, and ⌘Q / Alt+F4 already exist). The table is written per platform; adding Linux, Android or iOS later only adds entries (mobile has no keyboard shortcuts; the back gesture is mapped separately by the platform layer). These keys do not go into the global shortcut table and do not conflict with plugin bindings (nobody uses Ctrl+W yet).
3. Synthetic double-click in the playlist gets stuck in the drag state (existing bug no. 5): noted; the user will test with a real mouse later.
(2026-10-22 07:50–08:18, macOS, release build of branch `rebuild/store-ui` dc58f40)

The user authorized starting directly. The control group is a build of main 8bc172b (Slint 1.16).

**Slint 1.18 regression (research report §4.3)**: no regressions caused by the upgrade found.
- Passed: GPU playback; software rendering (including fullscreen 4:3); reading the video area right after showing/hiding the playlist (at 0.5× the window is 960/1280 wide, right the first time); fullscreen sidebar pop-out in sync with the bottom bar; Space still works after the bottom bar auto-hides (#11079 fix in effect); entering and leaving picture-in-picture; all 8 pages of the options window open; playlist ComboBox does not crash after changing a value and closing/reopening, and the value is kept; the "Close" button (`close-window` rename); Esc and "Cancel" in the "Add Link" window (`cancel` rename); drag reordering; the Markdown example (amber italics, grey strikethrough, inline code as one capsule including Chinese, code blocks, tables, local and remote images, scrolling); Chinese and English subtitles; NES run and stop; the About window shows Slint 1.18.1.
- Fixed by 1.18 in passing: the old NES tool window had only a title bar and no content; the new one is fine.
- Font comparison (W4, for the user to decide): `assets/2026-10-font-ab.png`, left is the current forced Hiragino, right is `VESTRA_FONT_FALLBACK=system` (PingFang). Chinese is fine in both.
- Not tested: ⌥⌦ and dead keys (need a real keyboard), mouse wheel on long lists, Windows.

**Existing bugs present in both old and new versions** (not caused by the upgrade):
1. window: after a video finishes and plays again, size, mini and picture-in-picture under "Screen" stay greyed out ("no video"). Cause: `video-size.ts` clears the size when playback finishes and does not probe again on replay.
2. window: the description text on the "Window" settings page wraps to two lines and the second line is clipped.
3. nes: after clicking "Stop" in the NES window, the OSD shows "Open failed: producer transport closed" (stop in the bottom bar does not).
4. plugins / info: the "Source" column in the plugin management list and the About window says only "Installed" for both repository and local-file plugins, without distinguishing them; the About window also says "Installed" for built-in plugins.
5. playlist: a synthetic double-click on a list item enters the drag state and does not play, until the next click (same in the old version). Possibly a synthetic-event problem; the user needs to confirm with a real mouse.
6. ⌘W does not close tool windows (product question, left to the user).

**Store acceptance (design §8.2)**:
- Passed: 1 open the store, official repository signature check and list; 2 details, permissions, license; 4 install YouTube from the official repository; 6 install from file (sideload warning, source "Local file"); 7 cross-source replacement (new instance, old data kept); 7b switching back to the local file reconnects the original instance c053abec and its data; 9 built-in plugins can only be disabled; 10 add a local third-party test repository (warning and "unofficial" mark); 11 update detection, update, rollback; 12 revocation auto-disables + notice + "Enable Anyway" risk confirmation + not disabled again afterwards; 13 incompatible versions cannot be installed; 14 disabling the store removes its settings page, re-enabling restores it.
- Bugs (for the store author):
  - S1 (high) README and changelog of official plugins always show "Could not load description: size does not match". Cause: Pages returns gzip, and `net.ts:133-136` compares the compressed Content-Length (647) with the decompressed size (892). The test's fake network does not compress, so it was not covered.
  - S2 the third line of each list row (the one-sentence summary) is half covered by the next row.
  - S3 after rollback the button says "Roll back to 0.2.0" (the newer version); it should be something like "Restore to".
  - S4 the status column of the "Versions" page is too narrow; "Incompatible: requires …" and "Current, revok…" are truncated.
  - S5 when reinstalling from the original source the confirmation still says "after replacing, the new plugin uses new empty data", while it actually reconnects the original data.
  - S6 the "Add repository" input and button on the settings page are too tall.
- Not tested: 3 opening links, 5 offline cache, 8 Plugin Workshop install (needs a paid model), dragging a `.vxp` onto the video area, install from link, 15 Windows.

**Retest (2026-10-22 08:56–09:03, post-merge build, SDK 1.1.3)**: store S1 (official README loads) and S2 (all three list lines visible) pass; ⌘W closes the options window, the store, and "Add Link" with focus in the input, and the main window does not respond; system fonts in effect; 0.5× available after replay; NES stop no longer reports an error; both description lines on the "Window" page visible; the source column shows "Built-in" and "Repository". The capture test flakiness is fixed (`rebuild/capture-flaky`, 80 concurrent runs with 0 failures). All checks pass (cargo test 302, 17 plugins, test:sdk 11/11, portability).
Remaining small issues (no functional impact, fixed separately after merge): S6 the bottom edge of the "Add repository" input overlaps the warning text below it; after switching to system fonts, wrapped grey description text has tight line spacing (the "Window" page, the store settings page, the store README).

## To do (after this round)
- **Real-device Windows acceptance (S6, when the user has a machine)**: Chinese fallback font, Consolas monospace, Ctrl+W / Ctrl+F4 closing tool windows (F4 alone is still stop), store checklist item 15 (install, sideload, cross-source replacement, the yes/no box for "Also delete data" on uninstall), GPU rendering and picture-in-picture fallback, all menus and plugin features (workflow §6.2).
- ~~UI: S6 input overlapping warning text; tight line spacing in wrapped descriptions~~: fixed (cc71fec, `line-height-factor: 1.25`), confirmed on a real device. Markdown body text (StyledText) has no line-height property in 1.18, so the store README line spacing is left as is for now; later a line-height variable can be added to Theme and shipped with the SDK.
- ~~Plugin repository: resource hashes, monotonic `generated`, multiple signatures, append-only `revoked.json`~~: done and released 2026-10-22 (plugin repository 68ff01b, live `generated` = 1, signature passes). The matching store changes are done (`rebuild/store-ui` dc58f40, tests 58/58; coordinator smoke test against the live index: signature passes, generated=1 recorded, YouTube package sha256 matches, second refresh 304).
- ~~Plugin repository youtube test~~: fixed (29ef948). Original: "download shows progress while running and the saved file when done" failed once intermittently on Windows + Node 22.18 (CI run 39678880495, dbd7e7d); the same test passed before and after. Handed to the youtube author.
- Binding loop warning on `sidebar-actual` in the main window (see above).
- ~~SDK: `StreamHandle.stop()` order~~: SDK 1.1.3 now closes the source first, and the host gives a 0.5 second grace period on disconnect (ee42ec1). Original: `StreamHandle.stop()` closed the data connection before the source, the host treated the disconnect as a load failure, and plugins that generate streams showed "Open failed: producer transport closed" when stopped. nes worked around it (`rebuild/plugin-fixes` bdb1dd3); the SDK should close the source first, and the host could also consider not treating a disconnect during playback as a load failure. Handed to the maintainer (after the font/shortcut task), with SDK 1.1.3.
- For the user to test: whether a real-mouse double-click on a playlist item plays it (a synthetic double-click gets stuck in the drag state).
- ~~The Docs workflow in the SDK repository fails when a tag is pushed~~: after the CI rework docs are only deployed from main (SDK 8384e03). Original: the Docs workflow failed when a tag was pushed (the Pages environment only allows main; this has been the case since v1.1.1, and the run on main deploys normally). Docs should only run on main.
- ~~reference §6.4 font notes~~: consistent after W4 removed the forced font (854d60d).

## CI rework (2026-10-22, preparing for the S7 release)

All three repositories were changed per `doc/process/ci-research.md` §10 to "reusable checks + three entry points: push to any branch / main / release", with `ci-ok` as the only required check; see `doc/process/ci.md`. Merged: player 13e2cb5, SDK 8384e03, plugin repository 414e8be; the first runs on main were all green.
- Measured (player, cold cache): TS-only change about 7 billable minutes; Rust change (adds Windows) about 25; full (adds macOS) about 46.
- CI ran the host tests on real Windows for the first time and 24 failures were fixed (directory flushing, `\\?\` paths, `/` inside packages); ttf-parser replaced by skrifa; the SDK's Windows tests used to always hang until timeout, now fixed.
- Set by the user: main rulesets on both public repositories (`ci-ok` required, no force-push or deletion); approvers for the SDK `vestra` environment (the coordinator changed the `v*` rule from branches to tags). The player is private, and the free plan cannot set branch protection or environment approval.
- **User to do**: after `npm login`, run `npm trust github @vestra-player/sdk --file release.yml --repo vestrasys/vestra-player-sdk --env vestra`. After the next SDK release confirms provenance, disable token publishing and delete `NPM_TOKEN`.
- Undecided: whether Windows package path rules reject `:*?"<>|` and reserved names (changes the package format, decided with S6); mirroring the mpv-dev download source to vestrasys; CODEOWNERS.

## Record
