# 2026-10 round: our own visual identity

Started 2026-10-22. **This document is the only progress source for this round.**

## Goal

Keep the current interaction feel (button and menu sizes, positions and the menu layout all stay the same), remove the traces of imitating the reference player, and create our own character; before open-sourcing, clean every reference trace out of the repository, including the git history.

## User rulings (2026-10-22)

1. Delete the second "classic" theme (an obvious imitation).
2. Clean up thoroughly before open-sourcing, including rewriting the git history.
3. The menu layout stays the same; interaction sizes stay the same.

## Current state (inventory 2026-10-22)

- The icons are not copied SVGs: the play bar icons and others are Paths hand-written in Slint, but their shapes were drawn after the reference player.
- Themes: `THEMES` in `src/host/src/theme.rs` contains the second theme id and 22 reference colours; the preferences plugin's "Settings ▶ Appearance ▶ Vestra / Classic"; the public SDK type `ThemeId` contains the second theme id (the SDK's `types.ts`, `appearance.ts`, `testing.ts`, `wire.ts`, `slint/vestra/*`, `docs/concepts/ui.md`).
- Player repository: 85 files mention the reference (51 under `src/`, the rest in `doc/`); `doc/product/research/` has strings, menu trees and resource tables extracted from the reference player's installer (1.6 MB); 9 commit messages mention the reference.
- SDK repository (already public): 9 files and 1 commit message mention the reference; the published npm packages (1.0.0–1.3.0) have it too; npm cannot withdraw content, it can only be removed in later versions.
- Plugin repository: clean.

## Division of work

- **A visual design** (proposals only, no product code changes): 2–3 icon styles + colour schemes, geometry exactly the same as now, comparison images for the user to choose from.
- **B cleanup** (in parallel with A, does not depend on the design decision): delete the second theme (host, preferences plugin, SDK `ThemeId`; old settings migrate to vestra automatically); remove the reference's name from code and comments; delete the extracted material under `doc/product/research/`; rewrite the benchmarking documents as our own interaction specification; clean up the SDK the same way and release it.
- **C rewrite git history** (last): after A and B are merged, use `git filter-repo` to remove the extracted material from history and rewrite the reference's name in commit messages; make a full mirror backup first; force-push all branches and tags; rebuild every local worktree. Same for the SDK repository (a public repository: existing forks/clones cannot be recalled, and the content of old npm versions cannot be changed).

## Progress

- [x] A design proposals → user chose (2026-10-24): **default A "V-Line rounded line × Aurora", optional theme A′ "Aurora Light"**; fully replaces the old shapes. Proposals and comparison images: `scratchpad/identity/` (proposal.md, compare*.png, icons-v-line.json, palette-aurora*.json)
- [ ] A implementation (icons, theme colours) + pixel comparison acceptance on a real machine
  - [x] Implementation: branch `identity/look` (player 0.4.0); SDK 1.5.0 branch `identity/look` (`ThemeId` adds `"vestra-light"`, `@vestra/icons.slint`), PR waiting to be merged and released. Notes and final screenshots in ["Implementation" in proposal.md](../../product/identity/proposal.md)
  - [x] Interaction geometry check (the interpreter reads element positions, compared before and after the change): identical; also fixed the defect where the playlist's six icon buttons had a 0×0 click area
  - [x] Player dependency upgraded after the SDK 1.5.0 release (main 2848111)
  - [x] Coordinator acceptance on a real machine (both palettes)
- [x] B cleanup (player, SDK) + SDK release: SDK 1.4.0 published; player main e17ec0e, 0 mentions in the whole tree
  - [x] Player: branch `identity/cleanup` (second theme deleted, old settings migrate to vestra; code, comments and documents cleaned up; new documents `ui-spec.md`, `interaction-spec.md`, `feature-catalog.md`)
  - [x] SDK: 1.4.0 (`ThemeId` has only `"vestra"`), PR vestrasys/vestra-player-sdk#10, waiting to be merged, tagged and released
  - [x] Player dependency upgraded after the release
- [x] C history rewrite (player, SDK): force-pushed 2026-10-24
  - [x] Dry run (2026-10-24): mirror backup and bundle, `git filter-repo` rewrite, verification (0 mentions in all objects of all refs, same number of commits). Scripts in scratchpad `history/` (`prepare.sh`, `push.sh`, `replacements.txt`); rerun `prepare.sh` before pushing
  - [x] Force-pushed after user confirmation (2026-10-24): pause Actions and the SDK ruleset → push → restore → redeploy the documentation site; local repositories switched to the new history in place; old worktrees, old branches and the old SDK clone deleted; 0 mentions in all objects. Full backup before the rewrite: `~/vestra-history-backup-20261024/` (bundle + SHA256SUMS)
  - [x] npm: 1.0.0–1.3.0 unpublished (done by the user, 2026-10-24); 0.0.0-stage, 1.4.0, 1.5.0, 1.5.1 remain
  - [ ] Old commit SHAs cited in documents are not rewritten (a mapping table can be obtained by rerunning filter-repo from the backup); refs/pull of the SDK's merged PRs still point to old commits; contact GitHub Support if needed
  - What remains on GitHub: refs/pull of the SDK's merged PRs still point to old commits, and only GitHub Support can clear them; npm 1.0.0–1.3.0 contain mentions and can be unpublished within 72 hours (the windows close one by one from 10-24 16:25Z), after that they can only be deprecated

## Record

- 2026-10-22: the user considered making the SDK and plugin repositories private (worried that the public API exposes the architecture and that someone publishes first). Assessment: it would need a Team plan, moving plugin packages to Pages, dropping npm provenance and plugin package attestations, and narrowing the CI matrix; 1.0.0–1.3.0 on npm are already public and cannot be recalled. **The user decided not to make them private for now**; both repositories stay public.
