# Round: plugin process supervisor (blackboard)

Coordinator record and blackboard for the supervisor project. Design: [supervisor.md](../../architecture/supervisor.md). Procedure: [supervisor-loop.md](../supervisor-loop.md). Started 2026-11-02 from main 0bc4d35d (host 0.6.5, #140, #141).

## User rulings (2026-11-02)

- One supervisor for all plugin processes; no lazy vs non-lazy modes; on-demand scheduling becomes the default (B6).
- It is process supervision modelled on systemd (not kernel CPU scheduling); module `supervisor`; our extra state is `standby`; wire values unchanged (`idle`).
- Limits and timings follow Chrome's precedent; process budget formula from Chrome; heap cap 512 MB per own process (ours, revisit if a legitimate plugin hits it).
- Built-ins never count against the installed package limit (#140).
- Build on the current implementation (no rewrite): keep the wire protocol, registries, replay and the 19 history amendments.
- All 16 open design questions answered by rulings, code, plan or precedent (supervisor.md §12.1).
- Run as a self-paced loop while the user is away; temporary role "spec test author" approved.

## State

| Batch | State | Branch / worktree | Agents | Notes |
| --- | --- | --- | --- | --- |
| B0 | not started | | | |
| B1 | not started | | | |
| B2a | not started | | | |
| B2b | not started | | | |
| B2c | not started | | | |
| B2d | not started | | | |
| B3 | not started | | | |
| B4 | not started | | | release after |
| B5 | not started | | | |
| B6 | not started | | | release after |
| B7 | not started | | | |

## Design questions (from spec tests or reviews)

None yet.

## Open items

- Known bugs to pin in B0: Book entry leaked when spawn fails after `activations.begin` (`load_package`); `Host.cleanup` only grows; disable then immediate enable can run two processes on one data directory (D13).

## Log

- 2026-11-02: design approved; supervisor.md, this record and supervisor-loop.md committed on `supervisor-design`.
