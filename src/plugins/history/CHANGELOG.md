# Changelog

## 0.1.5

- Stopping the plugin (deactivation, quitting the player) no longer fails with "command recent.item.0 is already declared" when a rebuild of Open ▸ Recently Played was due at that moment (a record or a resume point had changed within the last 0.5 s). The rebuild ran while the old submenu was still being removed and declared its commands again; the submenu now lives in a scope of its own that is released after the pending rebuild is cancelled, and a rebuild that was already under way stops there. The last position is still written on stop as before.
- Tests: the store files are read once written (polled) instead of right after a fixed wait, so a loaded machine does not read them before the write ends.
