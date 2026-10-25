import assert from "node:assert/strict";
import test from "node:test";
import { startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

/**
 * Fullscreen bar on hover only (ui-polish round, request FS-BAR-1): the host
 * decides when the fullscreen bar shows; this plugin's share is never to hold
 * it up from the keyboard. Key actions announce on the OSD but take no
 * interaction lock; only a real drag on the bar does, and it is released.
 */
const directory = new URL("..", import.meta.url);
const start = (host: TestHost) =>
  startPlugin(activate, { directory, host, locale: "en" });

function fullscreenWithMedia() {
  const host = new TestHost();
  host.openElsewhere("/movie.mp4");
  host.setSource({ duration: 600, position: 120 });
  host.setMainWindow({ fullscreen: true });
  return host;
}
async function until(predicate: () => boolean) {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error("plugin did not settle");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}
const dragging = (locks: unknown) =>
  (locks as { dragging?: boolean } | null)?.dragging === true;
const held = (locks: unknown) =>
  !!locks &&
  Object.values(locks as Record<string, unknown>).some(
    (value) => value === true,
  );

test("FS-BAR-1 keyboard seeks and speed in fullscreen show the OSD but never lock the bar", async () => {
  const { host, stop } = await start(fullscreenWithMedia());
  try {
    const bar = host.mounted();
    const before = host.sent("ui.interaction").length;
    await host.command("forward-2").run();
    await host.command("backward-2").run();
    await host.command("rate-15").run();
    await host.command("jump-start").run();
    assert.ok(
      host.osd.some((text) => text.startsWith("Forward 30 s:")),
      host.osd.join(" | "),
    );
    assert.ok(
      host.osd.some((text) => text.startsWith("Back 30 s:")),
      host.osd.join(" | "),
    );
    assert.ok(host.osd.includes("Speed: 1.5x"), host.osd.join(" | "));
    // No lock was taken (a lock would keep the hidden bar on screen).
    assert.equal(held(bar.locks), false);
    assert.ok(
      host
        .sent("ui.interaction")
        .slice(before)
        .every((locks) => !held(locks)),
    );
  } finally {
    await stop();
  }
});

test("FS-BAR-1 a drag on the bar holds it while dragging and lets go on release", async () => {
  const { host, stop } = await start(fullscreenWithMedia());
  try {
    const bar = host.mounted();
    bar.emit("dragging", true);
    await until(() => dragging(bar.locks));
    bar.emit("seekChanged", 0.5);
    bar.emit("dragging", false);
    await until(() => !held(bar.locks));
    bar.emit("volumeChanged", 40);
    await until(() => dragging(bar.locks));
    bar.emit("volumeReleased", 40);
    await until(() => !held(bar.locks));
  } finally {
    await stop();
  }
});
