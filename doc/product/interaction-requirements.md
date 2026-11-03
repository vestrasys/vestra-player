# Product interaction requirements

Product interaction constraints set by the user; the design and implementation of every plugin must meet them. The plugin development process is in the [community workflow](../../agentic-workflow/agentic-workflow.md); the concrete interaction in each state (keys, mouse, menus, window behaviour) is in the [interaction specification](interaction-spec.md).

## Scope decisions (2026-10-18)

- NES (since 2026-10-27 the store plugin VestraNES, not built in): a real emulator outputs the game picture and sound to the player; keyboard input goes to the game only while the VestraNES window is in front (its own window, no player API involved; fixed mapping), so the player's shortcuts are never taken; save / load states are deferred. Policies such as the save format, ROM verification and reset belong to the NES plugin, and the public API contains no NES-specific concepts.
- YouTube (playback and download) and Bilibili (playback) are separate plugins. The playlist can add their links and play them; site parsing, network requests and download policy stay in the site plugins, which cooperate through the public source resolution capability, and plugins do not reference each other.

## 1. Menus are organised by user domain

First build the menu structure of the whole player by functional domain, then assign plugin entries. The plugin split is not the menu split; entries must not grow separately just because different agents implemented different modules.

- Video parameters, video presets and the corresponding resets all go in the video domain.
- Audio parameters, the equaliser, delay, audio presets and the corresponding resets all go in the audio domain.
- Subtitle sources, tracks, style, sync and translation all go in the subtitle domain.
- Entries such as snapshots and clip export must have their place stated explicitly in the design; the same feature must not be spread over several top-level menus.
- Playback, files, the playlist, emulators, plugin management and the workshop likewise get their place decided first and are then implemented by plugins; no empty menus are created in advance for categories that have no need yet.

These domains can have submenus and several commands; a plugin is not required to have only one entry. "The same plugin can register several commands" must not be read as permission to scatter commands across domains at will. For example, "reset audio" stays in the audio domain and must not also hang in a general tools menu.

The same command can also have a suitable button or shortcut, but they must share state and behaviour and must not form two contradictory sets of settings. The global menu table records the single main menu path and any other necessary entries; where a duplicate menu entry is really needed, the user scenario must be stated, and it is added only after a unified review.

## 2. UI and feature ownership

Playback behaviour and the playback control bar are provided by default plugins; the Rust core manages windows, shared playback facilities, component mounting and lifecycle. Always present does not mean the feature implementation must go into the core.

Opening a tool UI such as audio must not replace the playback control bar; while a tool is shown, the user can still perform the play, pause and seek operations the design promises. Showing, hiding and closing a tool are different events from installing, running and disabling a plugin; closing a tool UI must not be treated as undoing its processing effect.

There are three forms of UI: a separate tool window, a resident control mounted in the bottom bar, and a panel mounted in the sidebar. Each plugin chooses by purpose; before adding a UI, confirm that it can reasonably coexist with the main picture, the resident controls and other tools.

Unified interaction rules are expressed by the product design; the core must not offer a dedicated layout or business backdoor by recognising a particular plugin's name. Reuse Slint's layout, widget and style capabilities instead of building another general UI framework.

## 3. Playback continuity is a real requirement

- When dragging picture or audio parameters continuously, each parameter event must not be implemented as restarting the whole playback session; the commit, waiting and failure behaviour of parameters must be explicit.
- Opening or keeping a context menu open must not lead to sound continuing while the picture freezes.
- Showing / closing a tool UI should not pause, seek or clear the playback state on its own.
- Operations that may affect the event loop, such as file selection and first-run install screens, must have their playback and state saving behaviour stated in the design and verified against reality in the later implementation.

These are constraints that the shared playback facilities and every plugin must meet. Correct output from an offline filter, a single successful request or a UI that compiles is not enough to prove these requirements hold; they must be verified by operating the real UI.
