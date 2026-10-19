# Two independent Slint contributions (source candidate)

`bar/bar.slint` and `sidebar/sidebar.slint` are separate example packages. They do
not import one another; both export a local `Bridge`, demonstrating owner aliases.
They are UI contributions only, not runnable SDK plugins or released manifests.

The maintainer can add `mod composition;` in its integration change and call on
the UI thread (paths here are relative to the repository root):

```rust
use composition::{CompileSpec, Primitive, Slot};
let specs = vec![
    CompileSpec {
        owner: 101,
        file: "src/examples/composition-demo/bar/bar.slint".into(),
        component: "PlaybackBar".into(),
        global: Some("Bridge".into()),
        slot: Slot::Bottom,
    },
    CompileSpec {
        owner: 202,
        file: "src/examples/composition-demo/sidebar/sidebar.slint".into(),
        component: "SourceSidebar".into(),
        global: Some("Bridge".into()),
        slot: Slot::Sidebar,
    },
];
let (sender, receiver) = std::sync::mpsc::channel();
let shell = composition::compile(&specs, sender)?;
composition::set_global(&shell, &specs[0], "status", Primitive::String("Ready".into()))?;
composition::set_global(&shell, &specs[1], "count", Primitive::Number(3.0))?;
// ComponentHandle must be in scope for show(); caller owns the event loop.
// shell.show()?;
// Poll receiver.try_recv() from the host's existing UI event dispatcher.
```

The host sets `video` using `Value::Image`, `diagnostic` using `Value::String`, and
`bottom-visible` / `sidebar-visible` using `Value::Bool`. Bottom spans the whole
window below video and sidebar. Sidebar is right of video. The existing backend
is independent of this shell and must survive rebuilding the UI.

Callbacks carry owner, local callback name, and primitive argument result.
`101 / set-paused(bool)` and `202 / select-row(string)` are requests; the caller
performs policy/control and writes confirmed facts back to the correct global.
Callback code never blocks waiting for host work. No source/queue policy is in
this module. All bridged callbacks must return void. Nonprimitive callback
arguments produce an explicit error event; property writes reject non-finite
numbers and interpreter type mismatches.

Aliases resolve exported names, not singleton identity. Each owner must use its
own package files and must not re-export shared stateful dependency globals;
canonical top-level file reuse is rejected. Package path containment and instance
liveness checks remain the package loader/router's responsibility.

Not built or visually validated by this helper. Integration must compile on the
locked Slint 1.16.1, show both contributions with a real video Image, verify each
callback/property pair and visibility, and prove failed candidate compilation
leaves the previous shell usable. This candidate does not implement commands,
dialogs, tool windows, full screen, hot reload policy, or a public SDK migration.
