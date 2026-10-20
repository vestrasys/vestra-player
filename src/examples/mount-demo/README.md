# Mount composition fixtures

`bottom/bar.slint` and `sidebar/sidebar.slint` are Slint fixtures for the host's
composition unit tests (`host/src/composition.rs`): two independent packages that
both export a `Bridge` global, compiled into one shell under owner aliases.

They are not plugins. The TypeScript specimens that used to drive them through
the removed flat `Host` API were deleted in SDK v2 P4 (see
`doc/rebuild/sdk-v2.md` §8); the product plugins `src/plugins/playback` (bottom)
and `src/plugins/playlist` (sidebar) show docked panels with SDK v2.
