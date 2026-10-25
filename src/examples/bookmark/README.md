# Bookmark example plugin

The plugin explained line by line in the [tutorial](../../../doc/plugins/tutorial.md): it records the current playback position and jumps back to it with one click.

In `src/`: `npm run plugin -- check|test|build examples/bookmark`.
To try it, in `src/host/`: `cargo run --bin vestra-host -- --plugin ../plugins/playback --plugin ../examples/bookmark`.
