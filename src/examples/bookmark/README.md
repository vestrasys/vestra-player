# Bookmark example plugin

It records the current playback position and jumps back to it with one click. The SDK site's [Bookmark example](https://vestrasys.github.io/vestra-player-sdk/documents/Example__Bookmark.html) walks through a plugin like it step by step.

In `src/`: `npm run plugin -- check|test|build examples/bookmark`.
To try it, in `src/host/`: `cargo run --bin vestra-host -- --plugin ../plugins/playback --plugin ../examples/bookmark`.
