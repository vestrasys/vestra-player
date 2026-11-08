# SDK refactor: measurement (H) and usage recount (I), 2026-11-08

Same machine (Ryzen 9 7940HS), Node 26.12.1, SDK 1.6.21 vs 1.7.2 interleaved, 3 runs each, all inside TestHost (no Rust host). Raw data and scripts: session scratchpad (bench-ext.mjs, fields.mjs, passcost.mjs, recount.mjs, ext-*.json).

## H. Passes and cost

Golden scenario (start → open → rate/paused/position ×3 → one command → unload), passes(full) per built-in, 1.6.21 → 1.7.2:
playback 20(19)→17(17) · playlist 17(11)→15(10) · info 8(3)→6(2) · subtitles 11(10)→8(8) · video 18(14)→14(11) · workshop 5(5)→4(4). Total passes 79→64 (−19%), evaluations 2161→1695 (−22%). skippedParts = 0 everywhere. The "state change" phase is unchanged; the savings are at start and unload.

Per-field test (10 changes each, 1.7.2): playback runs 2 full passes per change, subtitles 1 (listeners/manual update() force full passes — before the plugin.state sweep); playlist evaluates 0 for position/rate/volume (paused → full); video/info evaluate 0 (occasional safety pass). Full-pass net cost p50 µs: playback 95–110, video 67–74, subtitles 57–65, playlist 52–64, workshop 32–44, info 16–18.

Official bench (p50 ms, 1.6.21 → 1.7.2): diff 20k rows 1%: 0.96→0.55; 10%: 0.53→0.58; first full write 203→188; end-to-end 1%/10%: 75/76→74/76; 1000 full passes: 664→612; one state change → precise pass → ui.set: 0.04→0.03.

Extended bench (1.7.2, 20k rows, p50/p95 ms): move 1 row keyed diff 11/40 (index 0.36/3.2) but keyed end-to-end 43/54 vs index 123/175 and 111 B vs 1.11 MB sent; move 50 contiguous rows keyed e2e 38/61 vs 195/243 (2.3 KB vs 0.83 MB); 50 scattered 47/53 vs 123/168. One position tick with a 20k list present: precise pass 0.03/0.06 vs forced full 6.6/10.9.

Note: index-based end-to-end is ~40% slower on 1.7.2 than 1.6.21 (87→123, 137→195), keyed slightly (24→31); noisy laptop, but writer.ts changed ~500 lines in 1.7.0 — worth an SDK look.

Recommendation: keep row-move planning (real feature: playlist ids, move commands; removal turns a 0.1–2.4 KB patch into ~1 MB and 40 ms into 120–195 ms); keep field-level dependencies (precise vs full on position ticks: 0.03 vs 6.6 ms, ×200; plugin.state relies on the same mechanism). Candidate for removal after the sweep: `part()` partial skipping (skippedParts was 0 in every scenario) — re-measure after playback/info (now merged, #222) and the remaining built-ins.

## I. Zero-use recount (built-ins · plugins repo · examples · Workshop)

Delete list for the user's ruling (0 uses everywhere): `resolvers.fileTypes`, `resolvers.onFileTypesChange`, `commands.onKeysChange`, `timeline.onChange`, `appearance.onChange` (the last three replaced by `player.on("commands.keys" | "timeline.changed" | "appearance.changed")` — confirm the SDK state is updated before those listeners fire).
Keep: `when.open/ready/playing/paused/fullscreen`, `panels.bottom/adjust` (PanelSlot shared with sidebar/backdrop, 8 uses), `player.setPaused/previous`, `storage.session` (capture, shaderupscale use it), `video.upscaling.*` (video plugin), `TimelineLayer.setRanges` (capture), Snapshot fields, `AudioFilter.nodes` (audio; the inventory's regex missed the object-literal form).
Patch-only policy: mark `@deprecated` first; removal date is a separate ruling.
