# Window template preflight failure (2026-10-17)

Command: `cargo test --manifest-path src/host/Cargo.toml --bin vestra-host composition::tests::actual_compiler_accepts_shell_imports_models_and_both_mounts_without_a_window`.

The Slint Compiler test exited 101 after a final template edit. The following cargo build exited 0: Rust compilation alone does not validate dynamically interpreted Slint. These diagnostic messages were copied from the tool output; the original process output was not redirected to a file.

```
Cannot specify both 'height' and 'min-height'
Cannot specify both 'height' and 'max-height'
Cannot specify both 'height' and 'min-height'
Cannot specify both 'height' and 'max-height'
```

The four errors point at host.slint spans offset 1328 and 2904 (length 34): header and diagnostic text had explicit height plus min/max constraints. Fix: use equal min/max constraints without an explicit height. The affected actual Compiler test then passed (1 passed, 31 filtered out; 0.40s), followed by cargo build exit 0 (1.36s), tool session 29057. No source/processing suite was rerun for this template correction.
