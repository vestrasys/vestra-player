/** Consumer ordering test double only; never evidence of audio or backend playback. */
import assert from "node:assert/strict";
import test from "node:test";
import type { Host, UiCallback } from "../../../sdk/index.ts";
import { activate } from "../main.ts";
async function until(predicate: () => boolean) {
  const deadline = Date.now() + 1500;
  while (!predicate()) { if (Date.now() > deadline) throw new Error("consumer did not settle"); await new Promise(resolve => setTimeout(resolve, 5)); }
}
test("actual consumer submits release volume last and excludes overlapping mute", async () => {
  const calls: [string, number | boolean][] = [];
  const props = new Map<string, unknown>();
  let callback = (_event: UiCallback) => {};
  let release = () => {};
  const gate = new Promise<void>(resolve => { release = resolve; });
  const state = { eventSequence: 1, stamp: { generation: 0, intentVersion: 0 }, phase: "empty", source: null, paused: false, position: null, duration: null, rate: 1,
    capabilities: { pause: false, seek: false, rate: false, restart: false, close: false }, output: { volume: 100, muted: false } };
  const host = {
    source: { state: async () => state, submit: async () => { throw new Error("No player operation is simulated in this test"); } },
    playback: { set: async (property: string, value: number | boolean) => { calls.push([property, value]); if (property === "volume" && value === 80) await gate; } },
    commands: { register: async () => ({}), update: async () => ({}), onExecute: () => () => {} },
    ui: { onCallback: (fn: (event: UiCallback) => void) => { callback = fn; return () => {}; }, mount: async () => ({}), mountState: async () => ({ preferred: true }), setMount: async (key: string, value: unknown) => { props.set(key, value); }, interaction: async () => {} },
    events: { subscribe: async () => async () => {} },
  } as unknown as Host;
  const dispose = await activate(host);
  const emit = (name: string, value: number | string) => callback({ view: "$mount", callback: name, args: [value] });
  emit("volume-changed", 80);
  emit("volume-changed", 60);
  emit("volume-released", 35);
  emit("action", "mute");
  await until(() => props.get("can-mute") === false);
  assert.deepEqual(calls, [["volume", 80]]);
  release();
  await until(() => calls.length === 4 && props.get("can-mute") === true);
  assert.deepEqual(calls, [["volume", 80], ["muted", false], ["volume", 35], ["muted", false]]);
  emit("action", "mute");
  emit("volume-changed", 90); // outputBusy is set synchronously before awaiting the mute RPC.
  await until(() => calls.length === 5);
  assert.deepEqual(calls[4], ["muted", true]);
  assert.equal(calls.some(([property, value]) => property === "volume" && value === 90), false);
  dispose();
});
