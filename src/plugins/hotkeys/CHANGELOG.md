# Changelog

## 0.1.7

- Settings ▸ Mouse… offers the `Cmd` modifier on macOS only (⌘). On Windows `Cmd` is the Windows logo key and on Linux the Super key: the system takes most of their combinations and the player's menus never show such bindings, so the modifier row there is None / Ctrl / Alt / Shift. A `Cmd` mouse binding that is already in the binding table (written on a Mac, or by an older version) is kept: while one exists the `Cmd` radio is shown, so the binding can be seen and changed; the radio goes once no such binding is left and another modifier is chosen.
- Keyboard Shortcuts…: on Windows and Linux a key pressed with the Windows (Super) key is not recorded (the capture keeps waiting) instead of becoming a `Ctrl` binding. On macOS ⌘ and ⌃ still record `Ctrl`, which answers to both.
- The system is read once at start (`plugin.app.system()`); if it cannot be read, keys behave as on Windows.
