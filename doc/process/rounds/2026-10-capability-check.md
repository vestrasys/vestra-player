# 2026-10: Undeclared capabilities caught while writing and packing a plugin

Status: planned (user approved 2026-10-27). Owner: API maintainer (SDK and its tooling). Starts after refactor step C1 lands, because it reads the generated `src/host/capabilities.json`.

## Problem

A plugin that calls an API whose capability it did not declare in `manifest.json` compiles and only fails at run time: the host rejects the request with `capability` ("capability must be declared in manifest", `main.rs` dispatch). The host check stays — it is the security boundary, since a plugin can bypass the SDK — but authors should learn about the mistake earlier, without having to remember to run anything.

## Decision (user ruling)

One rule, shared by every entry point; no new command:

| Where | Triggered by | Author has to remember? |
| --- | --- | --- |
| IDE (red squiggle, quick fix "add `<capability>` to manifest.json") | a TypeScript language-service plugin shipped with the SDK and listed in the scaffolded `tsconfig.json` | no |
| `vestra-plugin pack` | pack already runs `check`; an undeclared capability fails it | no (needed to publish) |
| Plugin repository CI | the existing checks | no |
| Vestra Player AI (workshop) | its build loop already runs `check` | no |
| `vestra-plugin check` | optional manual run | optional |
| Host at run time | every request | — (stays as the security boundary) |

The rule maps SDK calls to capabilities from `capabilities.json` (methods per capability), so the IDE, `check`/`pack` and the host cannot disagree. Narrowing the SDK's types by manifest was rejected: it would break code that compiles today (a §1.6 signature change) for the same effect.

## Work (maintainer, after C1)

1. SDK: the call → capability mapping generated from the player's `capabilities.json` (or shipped alongside it), a shared checker module, the rule in `check` (and therefore `pack`), the tsserver plugin with the quick fix, the scaffold's `tsconfig.json`, docs, CHANGELOG ("Who must change: nobody"). Patch release.
2. Existing plugins (built-ins, plugins repository): run the new check; any finding goes to the plugin's author.
