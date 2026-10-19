# Processing token integration build

Author, 2026-10-17. Commands from repository root: `cargo test --manifest-path src/host/Cargo.toml --all-targets`, `cargo clippy --manifest-path src/host/Cargo.toml --all-targets -- -D warnings`, `cargo build --manifest-path src/host/Cargo.toml --bin vestra-host`; `npm run check` in src. All completed successfully. Host has 37 tests; composition-probe also compiles and runs reused module tests. Subtitle pure module is not yet included or claimed tested.

check-output.txt is the actual final tool continuation stdout/stderr chunk from session 28922 (the initial `Compiling vestra-host` line was in the earlier tool response). It includes test summaries, Clippy and build completion. TS check session 20608 completed exit 0 separately.

First all-target attempt found composition-probe missing the new processing_operations module import and two collapsible_if Clippy findings in window integration. The last cargo build itself was successful; it was not treated as overriding those failures. After adding the module path and following the lint fixes, the full commands above were rerun successfully. This build is not native/PCM/rollback-effect verification or a shared baseline release.
