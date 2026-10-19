# Current API usage and resolved requests

## Corrections to the first candidate

The earlier CAP-BURST and CAP-OPEN requests were not public-host gaps. This is a trusted Node plugin. Parent and maintainer confirmed direct Node I/O and `spawn(shell:false)` are available. No new host RPC is requested for directory reservation or system opening.

Implemented burst usage is `fs.mkdtemp(join(userChosenParent, "Capture-"))`, one immutable `snapshot.capture`, then sequential existing PNG jobs at fixed media times. Each completed public job records its published output; cancellation preserves published images and the worker owns temporary cleanup. Only empty plugin-created directories are removed. Single-file suggestions use actual `lstat`, while final conflict/no-overwrite protection remains the worker's publication transaction.

Implemented output opening reads the original completed job, validates matching destination and a real regular file, then invokes platform `open` / `explorer.exe` / `xdg-open` with an argument array and shell disabled. This is plugin business behavior, not arbitrary shell RPC in the host.

## Public operations actually consumed

- `source.state` plus source-state subscription: only current-source preparation is fenced by generation and intent. Accepted queued snapshots and explicit retained-input retry keep their original source.
- `snapshot.capture/state/release`: optional capture time means acceptance-time current position. Processed capture requires observed processing configuration revision and optional subtitle selection/presentation revision.
- `jobs.start/state/cancel/operation`: finite PNG/MP4 jobs, one worker at a time. One increasing operation ID sequence covers snapshot and job mutations.
- Maintainer confirmed `jobs.operation(id)` covers capture/release/start/cancel. Capture recovery validates snapshotId then reads public snapshot state; job-start recovery validates jobId and matching snapshotId. Unknown/expired stays unknown and is queried without submitting another write.
- The existing UI bridge recursively supports JSON arrays/structs, used for actual task rows in Slint. No new model RPC needed.

## Remaining concrete observations

`MediaJob` exposes lifecycle, publication flag and terminal facts, but no numeric decode/encode progress. Existing activity indication and actual published-count progress are adequate; there is no new progress API request. No public encoder preflight query exists; the plugin displays actual worker errors. Native focus/Escape behavior depends on ordinary tool-window host behavior and remains real acceptance work rather than a proposed capture-specific core API.
