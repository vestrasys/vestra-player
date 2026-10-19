# Independent production playback/playlist review — 2026-10-19

No confirmed defect was found in the requested boundaries. Independent production-entry protocol checks pass. This does not accept native interaction, actual SITE01 playback, backend CAS enforcement, or sound output.

Candidates: `playback-0.1.1-2026-10-19`, production main SHA256 `3811794d0ecd6ede84bdc7954bf96283bcf0a10dacd6d5ad4034deb6299c9353`; `playlist-site01-2026-10-19.1`, production main SHA256 `655c820583cd05174881f5a686bdf2a54c5691aa7fb2e394f65bebb26bb7c94e`. Markers identify candidates only. Current readable public `src/sdk/source.ts` SHA256 `c8e64f7110d1bb6a147fb13bc4d651e52737b981ac6a8a5b176b78851857f6d7` was inspected for SourceIntent, operation receipt, CAS stamp and error shape.

Independent evidence is `/tmp/vestra-acceptance-current/playback-playlist-independent.mjs` and `playback-playlist-independent-results.json`; Node command exit 0. Both actual production modules were imported without rewriting or stubbing product-local dependencies. All Host methods were simulated by this reviewer; author tests were not read or run as acceptance evidence.

| Boundary | Independent observed result |
| --- | --- |
| Playback cancel clears unsent draft | Hold first accepted seek at 20 seconds; queue 80-second draft; send seek-cancelled, then complete first operation. Only first seek was submitted; no queued seek escaped, and displayed progress refreshed to source position. |
| Final slider volume survives coalescing | Hold volume=10 write; queue changed=20 then released=27. Observed output order exactly volume10, unmute, volume27, unmute. Final source output and projection both volume27, muted=false. |
| Volume excludes mute | Mute callback during held volume work generated no extra output write. |
| Mute excludes volume | Hold mute write; send volume release55. No volume write occurred and prior volume27 remained. |
| Playlist SITE01 original URL routing | Adding `https://youtu.be/abcdefghijk?si=original` caused no open. Explicit Play submitted exactly the entered URL via generic source.submit(kind=open); no URL was converted to a filesystem path. |
| Unknown operation identity and exclusion | Inject code=timeout for first open. Repeat Play caused no second submission; recovery queried operation1, matching submission1. |
| Clear during unknown operation | Clear removed rows while recovery was outstanding. Returned receipt did not recreate rows/binding or submit close; simulated current URL source continued. |
| Natural-end automatic next CAS | Bound row A, then emitted natural end. Automatic row B open carried exact ended generation/intent stamp in expected. Injected stale response detached binding and caused no retry/new open; A remained current in simulated Host. |
| Clear after playback | All observed intents were open; clear submitted no close. |

Source evidence: playback `ui/bar.slint:77` handles PointerEventKind.cancel in its own branch, calling seek-cancelled without seek-changed; real pointer-event emission remains a native check. Playback `main.ts:170` rejects volume while outputBusy, `:253` rejects mute while volumeDraft exists, `:486` routes both changed and released to latest-value submission. `clearSeek` clears queued values and invalidates the gesture fence; already accepted seeks are not rolled back.

Playlist `main.ts:288` submits original row.source and optional expected stamp; `scheduleNext` checks current epoch/binding/natural-end/cycle/stamp before conditional open. Unknown-result polling queries the retained request.id. `queue.ts:146` clear only clears local rows/binding. Production import graphs consist of each product's own local helpers, public SDK type imports in TS and Node standard filesystem/path APIs; reviewed modules contain no cross-product imports or private product communication.

Limits: no GUI/Slint runtime pointer cancellation or release event was generated; no Host/Cargo/device/audio started; no provider extraction or real network media was opened; no native loading-event ordering or backend CAS race was exercised. Mount/dialog teardown is assigned to Host in the implementation and was not independently checked here. The independent checks cover specified successful and selected timeout/stale paths, not every command/error/unload interleaving. Full ten-product acceptance remains incomplete.
