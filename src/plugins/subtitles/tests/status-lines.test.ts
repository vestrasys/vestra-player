// The panel's status line after Reset, Subtitles Off and a refused style: only a subtitle that
// failed to load reads "The player cannot load this subtitle."
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { HostError, startPlugin, TestHost } from "@vestra-player/sdk/testing";
import activate from "../main.ts";
import { shownStatus } from "./status.ts";

const directory = new URL("..", import.meta.url);
const ZH = { kind: "embedded" as const, id: 1 };
const EN = { kind: "embedded" as const, id: 2 };
const text = (codec: string) => ({ styleSupported: true, styleReason: null, codec, text: true, streamIndex: null });
const CANNOT_LOAD = "The player cannot load this subtitle.";
type Sent = { marginPercent?: number } | null;

/**
 * Chinese (primary) and English (secondary) on a ready source. Like the player 0.8.0–0.8.2, the
 * host refuses a style whose margin is not a whole number of mpv pixels (`sub-margin-y` at 720
 * lines: 6% is 43.2) with `backend`.
 */
function strictHost() {
  const host = new TestHost();
  host.openElsewhere("/movie.mkv");
  host.subtitles.tracks.push(
    { target: ZH, title: "中文", language: "zh", ...text("subrip") },
    { target: EN, title: "English", language: "en", ...text("subrip") },
  );
  host.subtitles.selection = { stamp: { generation: 1, revision: 1 }, target: ZH, secondary: EN };
  host.handle("subtitles.style", (p) => {
    const style = p.style as Sent;
    if (style && !Number.isInteger(((style.marginPercent ?? 5) * 720) / 100))
      throw new HostError("backend", "unsupported format for accessing property (-9)");
    host.subtitles.presentation.revision++;
    host.subtitles.presentation.effective = style
      ? ({ owner: 1, manifestId: host.manifestId, revision: Number(p.revision), style } as never)
      : null;
    return structuredClone(host.subtitles);
  });
  return host;
}
const styles = (host: TestHost) => host.sent("subtitles.style").map((p) => p.style as Sent);

/** The plugin with the secondary position Above Main Subtitle and `style` remembered (null: none). */
async function start(host: TestHost, style: Record<string, unknown> | null) {
  const data = await mkdtemp(join(tmpdir(), "vestra-subtitles-status-"));
  const font = host.subtitles.presentation.fonts[0]?.id ?? "";
  await writeFile(join(data, "settings.json"), JSON.stringify({ version: 1, secondaryPosition: "aboveMain" }));
  if (style)
    await writeFile(
      join(data, "style.json"),
      JSON.stringify({ version: 1, style: { fontId: font, fontSize: 36, color: "white", outline: 2, marginPercent: 5, ...style } }),
    );
  const started = await startPlugin(activate, { directory, host, dataDirectory: data, locale: "en" });
  await host.command("tune").run();
  const panel = host.mounted();
  await host.settle(170);
  return {
    panel,
    stop: async () => {
      await started.stop();
      await rm(data, { recursive: true, force: true });
    },
  };
}

test("Reset restores the default style and says so; the restored style is not sent again", async () => {
  const host = strictHost();
  const { panel, stop } = await start(host, { bold: true });
  try {
    assert.equal(host.subtitles.presentation.effective?.style.bold, true, "the remembered style is applied");
    const before = styles(host).length;
    panel.emit("reset-all");
    await host.settle(300);
    assert.deepEqual(styles(host).slice(before), [null], "one write: the reset");
    assert.equal(host.subtitles.presentation.effective, null);
    assert.equal(shownStatus(panel.props), "Default style restored.");
  } finally {
    await stop();
  }
});

test("a style the player refuses is a failed operation, and Reset afterwards reads as a reset", async () => {
  const host = strictHost();
  const { panel, stop } = await start(host, { bold: true, marginPercent: 6 });
  try {
    assert.equal(styles(host).at(-1)?.marginPercent, 6, "the remembered style was tried");
    assert.equal(shownStatus(panel.props), "The subtitle operation failed.");
    const before = styles(host).length;
    panel.emit("reset-all");
    await host.settle(300);
    assert.deepEqual(styles(host).slice(before), [null], "the refused style is not sent again");
    assert.equal(shownStatus(panel.props), "Default style restored.");
  } finally {
    await stop();
  }
});

test("Subtitles Off in the Primary list reads Subtitles hidden, after a refused style too", async () => {
  const host = strictHost();
  const { panel, stop } = await start(host, { marginPercent: 6 });
  try {
    const before = styles(host).length;
    panel.emit("select-track", 0);
    await host.settle(300);
    assert.equal(host.subtitles.selection.target, null);
    assert.equal(shownStatus(panel.props), "Subtitles hidden");
    assert.equal(styles(host).length, before, "turning off sends no style");
    // Choosing a track again: the position is applied on the player's style, not on the refused one.
    panel.emit("select-track", 1);
    await host.settle(300);
    assert.deepEqual(host.subtitles.selection.target, ZH);
    assert.ok(styles(host).slice(before).every((s) => s?.marginPercent !== 6));
    assert.notEqual(shownStatus(panel.props), CANNOT_LOAD);
  } finally {
    await stop();
  }
});

test("a subtitle the player cannot load still says so", async () => {
  const folder = await mkdtemp(join(tmpdir(), "vestra-subtitles-load-"));
  const path = join(folder, "actual.srt");
  await writeFile(path, "1\n00:00:00,000 --> 00:00:01,000\nHello\n");
  const host = strictHost();
  host.handle("subtitles.commit", () => {
    throw new HostError("backend", "mpv could not add the subtitle");
  });
  const { panel, stop } = await start(host, null);
  try {
    host.dialogAnswers.push([path]);
    panel.emit("load-file");
    // The plugin reads the file itself before the upload: poll, `settle()` only waits for host calls.
    for (let i = 0; i < 100 && shownStatus(panel.props) !== CANNOT_LOAD; i++) await host.settle(20);
    assert.equal(shownStatus(panel.props), CANNOT_LOAD);
  } finally {
    await stop();
    await rm(folder, { recursive: true, force: true });
  }
});
