# Changelog

## 0.1.8

- The Keyboard Shortcuts page's two filter drop-downs, its “Ignore default shortcuts” check box and the Mouse page's modifier radios are the player's shared controls (`@vestra/widgets.slint` Choice, Check, RadioGroup) instead of the plugin's own copies, so they look and behave like the same controls elsewhere in the player: the drop-downs and the check box can be reached with Tab (a focused control shows an accent ring), a drop-down opens with Space, Enter, F4 or Alt+↓ and its list follows the arrow keys, the modifier radios are one Tab stop that the arrow keys move through, and a chosen radio has the accent ring. Clicking any of them still leaves the keyboard to the player's shortcuts. Requires player 0.6.0.
- The source filter's words (All sources, Player, Plugins not loaded) and the “None” modifier are now translated by the plugin; the translations are unchanged.

## 0.1.7

- Settings ▸ Mouse… offers the `Cmd` modifier on macOS only (⌘). On Windows `Cmd` is the Windows logo key and on Linux the Super key: the system takes most of their combinations and the player's menus never show such bindings, so the modifier row there is None / Ctrl / Alt / Shift. A `Cmd` mouse binding that is already in the binding table (written on a Mac, or by an older version) is kept: while one exists the `Cmd` radio is shown, so the binding can be seen and changed; the radio goes once no such binding is left and another modifier is chosen.
- Keyboard Shortcuts…: on Windows and Linux a key pressed with the Windows (Super) key is not recorded (the capture keeps waiting) instead of becoming a `Ctrl` binding. On macOS ⌘ and ⌃ still record `Ctrl`, which answers to both.
- The system is read once at start (`plugin.app.system()`); if it cannot be read, keys behave as on Windows.
