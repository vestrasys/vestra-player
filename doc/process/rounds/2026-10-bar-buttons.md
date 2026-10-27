# 2026-10 round: bottom-bar buttons contributed by commands

Started 2026-10-27 at the user's request: two new buttons left of the bottom bar's sidebar toggle, the Plugin Store, and further left the AI (Vestra Player AI). Owners: API maintainer (host 0.5.10, SDK 1.5.29), playback (0.2.5), store (0.2.3), workshop (2.2.8).

**This document is the record of the design discussion, the rulings and the result.**

## 1. The problem

The bottom bar is a built-in plugin (playback). The sidebar toggle was playback's own button reading the shared sidebar state (C7). Adding buttons for the store and the workshop inside playback would tie playback to two other plugins (their ids, their commands, whether they are installed or disabled). The user asked how the sidebar had been done and whether the bars should be core features instead of plugins.

## 2. Design (user approved 2026-10-27)

- **Commands contribute buttons.** `CommandOptions.button: { icon, order }`; `icon` is a name of the V-Line set or an image from `plugin.ui.image`; `order` 0–999, position only. The command must have a menu `path`, so every button has a menu twin.
- **The player renders them.** The host fills a `BarButtonsHost` global and draws the cells in the `BarButtons` component (`@vestra/bar-buttons.slint`); a click runs the command exactly like its menu entry. The bar plugin only places the component and gives it room, so a bar plugin can be replaced or restyled freely (no web view, the user's point) without knowing who contributes buttons. Low coupling: contributors know nothing about the bar; the bar knows nothing about contributors.
- **The sidebar toggle moves into the player's strip** (host command `host/sidebar`, "Toggle Sidebar", no default key: F6 belongs to the playlist), rightmost. Playback's own toggle and its bridge properties are removed. The left playback buttons stay in playback.
- **Limits:** 1 button per plugin, 8 per player.
- **Known weakness, accepted:** a bar plugin that does not place `BarButtons` hides every contributed button (VS Code has the same class of problem with contributed view actions). Mitigations: the docs ("Writing a bottom bar"), a host log line naming the bar and the hidden buttons, and a `vestra-plugin check` warning for a bar Slint file without `BarButtons`.

## 3. Coordinator decisions (standard practice)

- **Narrowing order.** The approved draft said "the highest order is hidden first", which would hide the Store before the AI although the acceptance expected the AI first. Decided: cells go from the strip's outer (left) end inward, the lowest `order` first and the toggle last; over 8 buttons the highest orders stay. `order` therefore means position only.
- **SDK versions.** Bar buttons take SDK 1.5.29; the docs-examples release that was also prepared as 1.5.29 moves to 1.5.30.

## 4. Result

| Part | Version | Notes |
| --- | --- | --- |
| Host | 0.5.10 | `bar_buttons.rs`, `bar-buttons.slint`, `host/sidebar`, tooltips with the first key, disabled reason, accessibility, log line |
| SDK | 1.5.29 | `button` option, `BAR_BUTTON_ICONS`, `BAR_BUTTON_LIMITS`, TestHost `barButtons`, `check` warning, docs |
| Playback | 0.2.5 | places `BarButtons`; widths in UI-BAR-10 |
| Store | 0.2.3 | `button: { icon: "store", order: 200 }` |
| Workshop | 2.2.8 | `button: { icon: "ai", order: 100 }`; vendored SDK 1.5.29 |

Product docs updated: `ui-spec.md` UI-BAR-09/10, `interaction-spec.md` G-18, `menus.md`, `identity/icons-v-line.json` (`store`, `ai`).

GUI check: by the user personally (user instruction 2026-10-27); the coordinator installs the build.
