/** Independent model/rowId specimen; no playback or queue policy is in Rust. */
import type { Host } from "../../../sdk/index.ts";

export async function activate(host: Host) {
  // Whole-model replacement is enough. rowId remains a plain stable field.
  let rows = [
    { rowId: "first", label: "First independent row" },
    { rowId: "second", label: "Second independent row" },
    { rowId: "third", label: "Third independent row" },
  ];
  await host.ui.mount({
    slot: "sidebar",
    file: "sidebar.slint",
    component: "RowsSidebar",
    global: "Bridge",
    properties: { rows, selected: "Select a row" },
  });
  let visible = true,
    revision = 1;
  const commandState = {
    enabled: true,
    checked: true,
    busy: false,
    reason: null,
  };
  await host.commands.register(
    "toggle-sidebar",
    revision,
    {
      title: "Toggle sidebar",
      path: ["视图", "Toggle sidebar"],
      shortcut: null,
    },
    commandState,
  );
  await host.commands.register(
    "reverse-rows",
    1,
    {
      title: "Reverse model rows",
      path: ["视图", "Reverse model rows"],
      shortcut: null,
    },
    { ...commandState, checked: null },
  );
  await host.commands.register(
    "fullscreen-sidebar",
    1,
    {
      title: "Toggle sidebar in current fullscreen",
      path: ["视图", "Toggle sidebar in current fullscreen"],
      shortcut: null,
    },
    { ...commandState, checked: null },
  );
  const offCommand = host.commands.onExecute((event) => {
    void (async () => {
      if (event.id === "fullscreen-sidebar") {
        const state = await host.ui.mountState();
        await host.ui.showMount(!state?.effective, "presentation");
      }
      if (event.id === "toggle-sidebar") {
        const state = await host.ui.mountState();
        visible = state?.preferred ?? visible;
        await host.ui.showMount(!visible);
        visible = !visible;
        await host.commands.update("toggle-sidebar", ++revision, {
          ...commandState,
          checked: visible,
        });
      }
      if (event.id === "reverse-rows") {
        rows = rows.toReversed();
        await host.ui.setMount("rows", rows);
      }
    })().catch(console.error);
  });
  const off = host.ui.onCallback((event) => {
    if (event.view !== "$mount" || event.callback !== "select-row") return;
    const rowId = event.args[0];
    if (typeof rowId !== "string") return;
    const row = rows.find((candidate) => candidate.rowId === rowId);
    if (!row) return; // Late or unknown row IDs never select a different row.
    void host.ui
      .setMount("selected", `${row.rowId}: ${row.label}`)
      .catch(console.error);
  });
  return () => {
    off();
    offCommand();
  };
}
