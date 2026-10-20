// Shared set-up for the plugin tests: the real SDK against the in-memory TestHost.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { startPlugin, TestHost } from '../../../sdk/plugin/testing.ts';
import activate from '../main.ts';
import { homebrew } from './homebrew.mjs';

const directory = new URL('..', import.meta.url);

/** Write generated ROMs (and a malformed one) to a scratch directory. */
export async function roms() {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'nes-plugin-test-'));
  const write = async (name, bytes) => {
    const file = path.join(dir, name);
    await fs.writeFile(file, bytes);
    return file;
  };
  return {
    blue: await write('blue.nes', homebrew(0x21)),
    red: await write('red.nes', homebrew(0x16)),
    bad: await write('bad.nes', new Uint8Array(100)),
    remove: () => fs.rm(dir, { recursive: true, force: true }),
  };
}

/**
 * Start the plugin. `intercept(method, params, next)` may delay or rewrite any
 * host request while keeping the TestHost's own behaviour behind `next()`.
 */
export async function start(intercept) {
  const host = new TestHost();
  if (intercept) {
    const request = host.request.bind(host);
    host.request = (method, params = {}) => intercept(method, params, () => request(method, params));
  }
  const started = await startPlugin(activate, { directory, host });
  // Before the first show() there is no window yet: read it as empty props.
  const window = () => host.windows.get('nes') ?? { props: {}, emit: () => assert.fail('window not open') };
  return { ...started, window };
}

/** Poll until `predicate()` holds (stream pacing runs on timers). */
export async function until(predicate, what = 'condition') {
  const deadline = Date.now() + 3000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise(resolve => setTimeout(resolve, 5)); // not settle(): a gated request never settles
  }
}

export const gate = () => {
  let open;
  const promise = new Promise(resolve => { open = resolve; });
  return { promise, open };
};
