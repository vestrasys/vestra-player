# Playback public consumer requests

## PB-K1 — ordinary main-canvas shortcuts (implemented by maintainer)

Consumer commands toggle-pause/Space, backward-5/ArrowLeft, forward-5/ArrowRight use existing commands.register and the same actions as menu/bar. Current cached SDK documents actual unmodified canvas routing with editor/menu/dialog/producer capture protection. Native key interaction of production 0.1.1 is still unverified by author.

## PB-K2 — modified file-open shortcut (still missing)

Maintainer reconfirmed on 2026-10-19: current host routes only the unmodified three keys above; any other shortcut string is display-only. This package now registers `open` at ["文件", "打开媒体…"] with shortcut:null instead of misleading Ctrl+O display.

Requested actual behavior: Ctrl+O on Windows/Linux and Meta+O on macOS dispatch that same public registered command, including while the bar is hidden. The handler remains commands.onExecute → single native dialogs.open → source.submit only on selected path, never on cancel. Host must preserve native editor handling and enforce enabled/busy, selector/modal ownership, and lifecycle cleanup. A platform-neutral declaration or explicit supported modifier spelling should be specified by maintainer. No plugin global OS hook, identity branch, or new input framework is requested.

This gap does not block menu/button file opening. It does block claiming the B1 modified shortcut complete.

## Native boundaries

New pointer cancel/release and volume mutual exclusion need native Slint and device verification. source.operation control acceptance is not proof of a displayed frame. Output uses existing playback.set(volume) followed by playback.set(muted,false), never restores an old loud value before lowering it. Fullscreen effective visibility remains host mount policy and public ui.interaction; menu check represents preferred visibility. Generic website resolver business is not implemented by this package.
