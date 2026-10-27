# Changelog

## 1.1.6

- Deinterlace **Auto** (the default) now uses the player's own deinterlacer (`{ kind: "deinterlace", when: "auto", output: "field" }`, player 0.5.4) instead of a `bwdif` filter in the chain. The player decides per frame from the decoder's interlaced flag, so progressive files (almost all modern video) get no deinterlacer and no filter: with the default settings decoding stays zero-copy where the renderer allows it, instead of copy-back on every file. Interlaced files and the interlaced parts of mixed files are still deinterlaced, with the decoder's hardware deinterlacer where frames stay on the GPU.
- Auto now deinterlaces **by field** (double rate: 25i plays at 50 fps), as mpv and most players do; the player's deinterlacer has no frame-rate output. The Output setting (By Frame / By Field) applies to **Always** only: under Auto it is greyed in the Deinterlace ▸ Output menu and in the window, with a tip that says so. The stored Output is kept and used again when Always is chosen; settings saved by earlier versions (Auto + By Frame) load as Auto at field rate.
- Always (`bwdif` on every frame, with the Output setting) and Never (no node) are unchanged. The "many frames dropped with field output" hint now watches Always + By Field only, where switching Output is a choice.
- Requires player 0.5.4 (`minPlayerVersion`).

## 1.1.5

- The colour `eq` filter is in the chain only while brightness, contrast, saturation or gamma differs from its default; it was always there, at neutral values. Any filter in the chain makes the player decode with copy-back (GPU → system memory → GPU), so untouched colours cost zero-copy hardware decoding (`d3d11va` on the Windows GPU build became `d3d11va-copy`) and made exact seeks on long-GOP files 3–5× slower. Moving a colour away from its default adds the node while playing, and the player rebuilds its filter stage once (and switches the decoder to copy-back, which replays from the last keyframe): a short hiccup on the first move, acceptable. A drag keeps the node until the release, so dragging through 0 updates it in place; the release, a key step, Reset (Q) or the Original preset remove it once every colour is back at its default, with the same one-time hiccup. Settings restored at startup with a non-default colour still add the node.
