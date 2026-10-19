# Native mount specimens — experimental candidate

`bottom/` and `sidebar/` are independent example packages. Each has its own
manifest, TypeScript entry and Slint file, with no sibling imports. Both export
`Bridge`; the host imports/re-exports each global under its owner-generation
alias. The manifest IDs only identify packages; no Rust behavior depends on them.

This candidate uses the maintainer's proposed `ui.mount` capability and methods:

```ts
await host.ui.mount({ slot: "bottom", file: "bar.slint", component: "SourceBar",
  global: "Bridge", properties: {} });
await host.ui.setMount("status", "Ready");
await host.ui.showMount(false);
await host.ui.showMount(true);
await host.ui.unmount();
```

One live owner contributes at most one slot. The finite shell has one bottom and
one sidebar contribution. `file/component/global/slot` are explicit request
metadata, not new manifest fields. A mounted global callback arrives through
`host.ui.onCallback` as `{view: "$mount", callback, args}`. JSON structs/models
are bounded by `ui_values`; callbacks use a bounded `try_send` channel. A full
channel rejects the callback and logs the overflow instead of waiting on the UI
thread. All bridged callbacks must return void.

The bottom specimen reads confirmed public source facts and requests pause with
source+intent CAS. The sidebar sends a real array-of-struct model to Slint's
ListView and validates returned rowId in TypeScript. It is not a product queue.
The examples do not open media automatically. Open a source with another public
consumer to exercise pause; the sidebar can be exercised without media.

The host template retains video-frame, diagnostic and the shared menu popup.
`composition::shell_source` uses these four exact comments in host/ui/host.slint:

- `// VESTRA-MOUNT-PROPERTIES` in the Window property area;
- `// VESTRA-MOUNT-CANVAS-BEGIN` immediately before the video Rectangle;
- `// VESTRA-MOUNT-CANVAS-END` immediately after that Rectangle;
- `// VESTRA-MOUNT-BOTTOM` at the end of the outer VerticalLayout.

The maintainer owns adding these markers and wiring the lifecycle/router. The
module compiles an unshown candidate; main must keep the previous usable shell
if preparation, compilation, callback binding or initial property binding fails.
Owner liveness/path containment and mandatory cleanup on stop/crash remain host
responsibilities. The legacy compile/Primitive probe bridge remains available;
its `video` property aliases the template's `video-frame`.

Not built, typechecked or visually exercised by this helper. SDK/router methods
and the capability above require the maintainer's integration. These specimens
do not establish a published common plugin baseline. Real validation must show
both mounts with advancing video, functioning menu popup, pause confirmation,
rowId callbacks, show/hide, owner cleanup and failed preparation preserving the
old shell. Alias names do not isolate shared singleton dependencies: packages
must not share a stateful global through a common imported file.
