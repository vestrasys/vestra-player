# Notes for the next release

User-facing notes of versions built since the last release that had no release of their own. The notes of 0.5.10–0.5.23 went into [v0.6.2.md](v0.6.2.md). They are material for the next `v<version>.md`, not a release page: `release.yml` reads only `v<version>.md`.

## Built-in plugins (no host change)

### Fixed

- **The Plugin Store and Vestra Player AI windows resize properly again.** The Plugin Store can be made as tall as the screen, can no longer be squeezed below 480 pixels, opens 640 high again, and its cards no longer jump while the window is resized: the card at the top left stays in the top row when the number of columns changes. The Vestra Player AI window can be made wider than 720 pixels again, no longer gets narrower than that, and opens 900 wide (Store 0.3.3, Workshop 2.2.47). The same hidden-element pattern was made safe in the About, Properties, Marks and Add Links windows, which were not affected (Info 0.1.17, Marks 0.1.11, Playlist 0.2.11).
