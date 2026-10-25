# Round 1: features without core dependencies + negotiation of core requests

Branch: `rebuild/features-r1` (from `main` 6ef174f). Basis: [feature requirements](../../product/feature-requirements.md) (below: "the requirements document"), [community workflow](../community-workflow.md) §3, §4 (especially §4.3 negotiation). 4 rounds in all; the plan is in §5.

## 1. Two tracks

- **Track A: plugin authors**. Implement the 29 items in the requirements document marked "depends on new core: no".
- **Track B: negotiation**. Take all core requests in Appendix A of the requirements document through §4.3; the conclusions decide the scope of rounds 2–4.

The two tracks do not wait for each other.

## 2. Author rules

- Write only in your own plugin directory (create a new directory for a new plugin); do not change the core, the SDK, sibling plugins or `doc/` (the coordinator updates the requirements document and the menu table).
- Implement according to the matching section of the requirements document; menu order and shortcuts follow the "Menus and shortcuts" part of each chapter in the requirements document. Anything outside the table must be explained in the report.
- If an item turns out to need a core change, skip that item, write a request in the report's "API requests" section according to workflow §4.1, and do the rest.
- Use `Theme` for colours and sizes; use `@vestra/widgets.slint` for controls.
- Tests: add tests for new behaviour, and update tests for old behaviour that changed.
- Done when: `npm run plugin -- check|build|test <plugin directory>` all pass. The report lists finished items, unfinished items with reasons, API requests, and documents the coordinator needs to update (menus, shortcuts).
- No commit, no push, no reset, no GUI operation.

## 3. Assignments (track A)

| Author | Plugin | Items |
| --- | --- | --- |
| A | playback | A02, A10, B09, B29, I06, I08 |
| B | playlist | A03, A13, C04, C07, C11, C17, C18, C21, C23 |
| C | subtitles, audio | A09, D02, D05, D06, D07; G09, G11 (subtitle sync changes to `.` `,` `/`, audio sync uses `>` `<` `?`; change both sides together) |
| D | capture, info | F03, F04, F07, F09; I03 |
| E | history (new plugin) | A12, B25 |

## 4. Negotiation (track B)

| Step | By | Output |
| --- | --- | --- |
| 1 | API maintainer | Reply to each request in Appendix A: an alternative without API changes and its cost, or the reasons for preliminary approval |
| 2 | Author representative (on behalf of each requesting plugin) | For items the maintainer did not approve, at most one rebuttal, with evidence |
| 3 | API maintainer | Ruling: reject with an alternative / accept after rewriting / accept; merge requests for the same capability; schedule into rounds 2–4 |

Recorded in [2026-10-features-negotiation.md](2026-10-features-negotiation.md).

## 5. Round plan

| Round | Contents |
| --- | --- |
| 1 | This document |
| 2 | Core batch one (playback, tracks, processing nodes, including moving basic playback actions to the core), then plugin integration |
| 3 | Core batch two (interface, windows, settings, bindings), mainly new plugins |
| 4 | Rendering rework (C-SIZE, C-GPU, picture-in-picture) |

The exact items of rounds 2–4 follow the negotiation conclusions. At the end of each round: static checks, all plugin tests, GUI testing, merge into `main` and push.

## 6. Record

### Author deliveries (2026-10-20)

| Author | Plugin | Done | Tests | API requests |
| --- | --- | --- | --- | --- |
| A | playback | A02, A10, B09, B29, I06, I08 (system commands gathered in `system.ts`) | 37 | None |
| B | playlist | A03, A13, C04, C07, C11, C17, C18, C21, C23 | 56 | None |
| C | subtitles, audio | A09, D02, D05, D06, D07; G09, G11 (plus a normalized entry for G10); sync keys swapped | 30, 22 | None |
| D | capture, info | F03, F04, F07, F09; I03 | 22, 9 | None |
| E | history (new) | A12, B25 | 21 | None |

All 29 items done, no API requests. All 12 plugins' tests pass (264 in total); `check`, `format:check`, `boundaries` pass.

### Negotiation (track B)

Rulings on 67 requests: 6 rejected with an alternative, 30 accepted after rewriting, 31 accepted; 0 author rebuttals (with measured data). Several general core capabilities were also adopted: clipboard, sleep prevention, confirmation dialogs, and `tools`, `storage`, `system` in the runtime layer. Because of the size budget the rounds were split into 2, 3a, 3b, 4a, 4b. Details in the [negotiation record](2026-10-features-negotiation.md) and the [round 2 task sheet](2026-10-features-r2.md).

### GUI testing (coordinator)

**Passed**:
- Open folder: natural sort order; hidden files, .app and txt are skipped.
- The list is restored across restarts.
- Search: filtered results keep their original numbers; a message when nothing matches.
- Time on seek bar hover.
- Brief info.
- Backspace jumps to the start.
- Reopen last file: the menu works.
- ⌘C copies the frame: the clipboard holds a PNG, no leftover file.
- Snapshot saved as JPEG.
- Sleep prevention: the `pmset` assertion appears and disappears correctly with play and pause.
- Recently played menu.
- Subtitle sync `.` `,`, audio sync `>`, the manual sync adjustment window.

**Fixed on the spot**:
- Overflow on the "Effects" page of the audio workbench (author C).
- At the default width the total duration was pushed out by the search box: ruled that when narrow the search box collapses into a magnifier icon (author B).
- Brief info line too long: use short codec names and limit the width (author D).

**Host defects, moved to round 2 batch A**:
- Option key combinations do not trigger (Alt+L, Alt+O).
- Ctrl+letter arriving as a control code is misread: Ctrl+Y is read as Shift+Tab, so the "Reopen" shortcut does not work; Ctrl+I, Ctrl+H, Ctrl+M, Ctrl+[ are also wrong.

**Remaining observation**: after closing the media, the video area still shows the last frame; left for the idle screen of round 3 (H12).

The menu table (menus.md) is updated in one go after the basic actions are handed over in round 2.
