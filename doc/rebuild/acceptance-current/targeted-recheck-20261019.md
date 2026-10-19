# Independent targeted recheck — 2026-10-19

Result: the two previously CLI-confirmed defects are **not reproduced** in the specified new candidates. Related install/rollback timeout recovery and trial exclusion also pass this independent contract harness. This is not full product acceptance.

Reviewed/executed revisions:

- NES production `dist/main.js` SHA256 `ed4f72bccb472ff98fb2b2238200e820fd87d000a80c2aa584a8f2ae4297e58b`.
- Workshop revision 2 `main.ts` SHA256 `ebf0a96888ddb0dbf0bf5fd7d5e2444fbc0b1931fef0b598a9da10f9ca07cf27` (executed with Node type stripping).
- Workshop production `main.js` observed SHA256 `03de48ae0053d5c3bc75ef953897c5316fa3f9892656e9433c9433d023631772`; not separately executed by this harness.

| Previously reported path | reproduced | Independent observed result |
| --- | --- | --- |
| NES Stop while prepare response is held | false | After Stop and releasing held response: no source submission, producer 1 released once, no Running status. |
| NES Stop after source accepted but before submit response returns | false | Late response causes same-generation close, socket close and producer release; no Running status. |
| NES cancellation after another source replaced accepted generation | false | Generation 8 is preserved while cancelling generation 7; no close submitted for replacement; socket closed and producer released. |
| Workshop metadata lookup failure locks busy | false | Injected first failure resets busy=false; second attempt reaches installed lookup again (two calls), and also resets busy. |
| Workshop rollback accepted-but-response-lost | false | Injected code=timeout causes query by exactly the original requestId; second callback submits no second mutation; committed result resets busy. |
| Workshop install accepted-but-response-lost | false | Same identity and exclusion facts as rollback; one install, queryId=requestId, busy reset after committed. |
| Workshop two rapid trial callbacks | false | Exactly one trial request; busy held during pending receipt; returned trial 51 retained and stop targets exactly 51. |

Evidence: `/tmp/vestra-acceptance-current/recheck-revision2.mjs` and `/tmp/vestra-acceptance-current/recheck-revision2-results.json`. Command `node /tmp/vestra-acceptance-current/recheck-revision2.mjs` exited 0. Harness derives from this reviewer's prior failing reproduction; no author test was used as evidence.

Boundary: all Host methods are simulated public calls. The install and trial paths use an explicitly substituted in-memory WorkshopJob.run dependency to reach validated controls without repeating Pi/compiler work; no candidate or product file was changed. Metadata and rollback checks use the actual unmodified Workshop entry module with no generation substitute. No real Host, Slint, GUI, audio, Cargo, package transaction, trial runtime or cleanup fence was executed. These results establish entry-module control-flow recovery and call identity, not actual native AV or managed transaction correctness. Core greens were not repeated, and no paid model ran. Existing full-ten-product acceptance gaps remain outside this targeted recheck.
