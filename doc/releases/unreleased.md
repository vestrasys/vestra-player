# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## For plugin authors

- **SDK 1.11.3** (see its changelog). Security fixes, additions, and controls that read the state first when a plugin keeps no copy current; nobody must change anything except a plugin test that relies on a host error the plugin does not handle.
  - `TestHost.strictErrors` (default `true`): a host error the plugin leaves to its log (no `try`/`catch`, no `plugin.onError`, also a shader or render failure the SDK reports beside a status) fails the test. A test that means it calls `host.expectError(code)` first; `TestHost.loggedErrors` lists them all.
- The Plugin Workshop is 2.2.50 and vendors SDK 1.11.3.
