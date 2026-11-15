/**
 * Card actions (Store v4, design store-v4 §3, owner rulings 2026-11-15): the
 * ⋯ menu on installed cards acts in place, a revoked card offers Uninstall…
 * only, and a card hides its Install button while its plugin installs and
 * shows the progress instead.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { BUILTIN, controls, FakeNet, info, OFFICIAL, openStore, props, publish, rows, start, until, yes } from "./helpers.ts";

const fromRepo = { kind: "repository", url: OFFICIAL } as const;

test("the card's ⋯ acts in place: Disable, then Enable from the text button, then Uninstall… asks first", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }]);
  const { host, store, stop } = await start({ net, packages: [BUILTIN, info("a.alpha", { version: "1.0.0", source: fromRepo })] });
  try {
    await openStore(host);
    await until(host, () => rows(host).some((r) => r.name === "Alpha"), "card");
    const key = rows(host).find((r) => r.name === "Alpha")!.key;
    const window = host.window("store");
    assert.deepEqual(controls(host, "Alpha").menu, ["disable", "uninstall"]);
    window.emit("cardAct", key, "disable");
    await until(host, () => controls(host, "Alpha").dim, "disabled");
    assert.equal(props(host)["detail-open"], false, "the grid stays");
    assert.deepEqual(controls(host, "Alpha").button?.id, "enable");
    window.emit("cardAct", key, "enable");
    await until(host, () => !controls(host, "Alpha").dim, "enabled");
    host.dialogAnswers.push(yes);
    window.emit("cardAct", key, "uninstall");
    await until(host, () => !store.busy && !host.packages.some((p) => p.id === "a.alpha"), "uninstalled");
    assert.equal(host.confirmTexts.at(-1)!.message, "卸载「Alpha」？");
    await host.settle();
    assert.deepEqual(controls(host, "Alpha").button?.id, "install");
    assert.deepEqual(controls(host, "Alpha").menu, []);
  } finally {
    await stop();
  }
});

test("a revoked card offers Uninstall… only; Enable Anyway… stays on its page", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "a.alpha", name: "Alpha", versions: [{ version: "1.0.0" }] }], {
    revoked: [{ id: "a.alpha", versions: "1.0.0", reason: "broken" }],
  });
  const { host, store, stop } = await start({ net, packages: [BUILTIN, info("a.alpha", { version: "1.0.0", source: fromRepo })] });
  try {
    host.dialogAnswers.push({ confirmed: false, checked: false });
    await store.refreshAll(true);
    await openStore(host);
    await until(host, () => controls(host, "Alpha").state === "已撤回", "revoked");
    assert.equal(controls(host, "Alpha").button, null);
    assert.deepEqual(controls(host, "Alpha").menu, ["uninstall"]);
  } finally {
    await stop();
  }
});

test("while its plugin installs, a card shows the progress instead of its Install button", async () => {
  const net = new FakeNet();
  publish(net, OFFICIAL, [{ id: "b.beta", name: "Beta", versions: [{ version: "1.0.0" }] }]);
  // A slow download, so the window can be read while it runs.
  const url = "https://plugins.test/download/b.beta-1.0.0.vxp";
  const route = net.routes.get(url)!;
  net.route(url, { ...(route as object), delay: 400 });
  const { host, store, stop } = await start({ net });
  try {
    await openStore(host);
    await until(host, () => rows(host).some((r) => r.name === "Beta"), "card");
    const key = rows(host).find((r) => r.name === "Beta")!.key;
    host.dialogAnswers.push(yes);
    host.window("store").emit("cardAct", key, "install");
    await until(host, () => controls(host, "Beta").state.startsWith("正在安装…"), "installing");
    assert.equal(controls(host, "Beta").button, null);
    assert.deepEqual(controls(host, "Beta").menu, []);
    await until(host, () => !store.busy && host.packages.some((p) => p.id === "b.beta"), "installed");
    await host.settle();
    assert.equal(store.installing, null);
    assert.equal(controls(host, "Beta").state, "✓ 已安装");
    assert.deepEqual(controls(host, "Beta").menu, ["disable", "uninstall"]);
  } finally {
    await stop();
  }
});
