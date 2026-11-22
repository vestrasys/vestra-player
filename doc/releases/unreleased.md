# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## Improvements

- **Less memory on Windows.** The player now uses Windows' newer segment heap, which returns freed memory to the system sooner: about 20–45 MB less private memory with CPU rendering (idle 188 → 161 MB, 1080p 263 → 238 MB, 4K 406 → 387 MB, with the adjust panels 464 → 429 MB), about the same with GPU rendering. Frame rate, start-up time and CPU use are unchanged. Windows 10 before version 2004 ignores the setting.
