import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { activate } from '../dist/main.js';
import { homebrew } from './homebrew.mjs';
const waitFor = async predicate => {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('Timed out waiting for test boundary');
    await new Promise(resolve => setTimeout(resolve, 2));
  }
};
for (const boundary of ['prepare', 'source-submit']) {
  test(`Stop while ${boundary} is pending revokes the attempt and releases its producer`, async () => {
    const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'nes-cancel-test-'));
    const rom = path.join(directory, 'original.nes');
    await fs.writeFile(rom, homebrew());
    let execute, unblock, reached = false, selected = false, pcm = 0, hasVideo = false;
    const released = [], intents = [], reports = [];
    const prepared = { producerId: 7, epoch: 1 };
    const host = {
      ui: { open: async () => {}, show: async () => {}, close: async () => {},
        set: async (_view, field, value) => { if (field === 'status') reports.push(value); },
        onCallback: () => () => {} },
      dialogs: { open: async () => ({ cancelled: false, paths: [rom] }) },
      commands: { register: async () => {}, onExecute: fn => { execute = fn; return () => {}; } },
      producer: {
        prepare: async () => {
          if (boundary === 'prepare') { reached = true; await new Promise(resolve => { unblock = resolve; }); }
          return prepared;
        },
        connect: async () => ({ video: async time => { hasVideo ||= time.sampleTime === 0; },
          audio: async (_time, audio) => { pcm += audio.length / 2; }, close: () => {} }),
        state: async () => ({ epoch: 1, acknowledged: true, health: 'ok', receivedFrames: pcm,
          hasVideo, active: selected, paused: false, consumedFrames: 0, pendingControl: null, error: null }),
        release: async id => { released.push(id); },
      },
      source: {
        submit: async intent => {
          intents.push(intent);
          if (intent.kind === 'openProvider') {
            selected = true; reached = true;
            await new Promise(resolve => { unblock = resolve; });
            return { status: 'completed', ticket: { stamp: { generation: 17, intentVersion: 1 } } };
          }
          selected = false;
          return { status: 'completed', ticket: { stamp: { generation: 17, intentVersion: 2 } } };
        },
        state: async () => ({ kind: selected ? 'producer' : 'media', stamp: { generation: 17, intentVersion: 1 } }),
      },
    };
    const cleanup = await activate(host);
    try {
      execute({ id: 'nes.open' });
      await waitFor(() => reached);
      execute({ id: 'nes.stop' });
      await waitFor(() => reports.includes('Stopped.'));
      unblock();
      await waitFor(() => released.includes(7));
      assert.equal(reports.some(value => String(value).startsWith('Running ')), false);
      assert.equal(selected, false);
      if (boundary === 'prepare') assert.equal(intents.length, 0, 'Cancelled prepare must not select a source');
      else assert.deepEqual(intents.map(intent => intent.kind), ['openProvider', 'close']);
      assert.deepEqual(released, [7]);
    } finally {
      unblock?.(); await cleanup(); await fs.rm(directory, { recursive: true, force: true });
    }
  });
}
