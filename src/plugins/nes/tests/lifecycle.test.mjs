import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { activate } from '../dist/main.js';
import { homebrew } from './homebrew.mjs';
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const waitFor = async predicate => {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('Timed out waiting for lifecycle boundary');
    await delay(2);
  }
};

test('initial AV prefill precedes source selection; later ACK gates streaming and stale cleanup preserves newer ROM', async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'nes-homebrew-'));
  const rom = path.join(directory, 'test.nes');
  await fs.writeFile(rom, homebrew());
  let execute, generation = 0, operationPolls = 0;
  const calls = [], closed = [], released = [];
  const primed = new Map();
  const host = {
    ui: { open: async () => {}, show: async () => {}, set: async () => {}, close: async () => {}, onCallback: () => () => {} },
    dialogs: { open: async () => ({ cancelled: false, paths: [rom] }) },
    commands: { register: async () => {}, onExecute: fn => { execute = fn; return () => {}; } },
    source: {
      submit: async intent => {
        if (intent.kind === 'openProvider') {
          const buffers = primed.get(intent.producerId);
          assert.ok(buffers.frames >= 2048, 'Source selection requires initial PCM');
          assert.ok(buffers.videoAtZero, 'Paused initial producer needs video at sample zero');
          generation++; operationPolls = 0; calls.push(`submit:${generation}`);
          return { status: 'pending', ticket: { stamp: { generation, intentVersion: 1 } } };
        }
        calls.push(`close:${generation}`); return { status: 'completed' };
      },
      operation: async () => { operationPolls++; calls.push(`operation:${generation}`); return { status: 'completed', ticket: { stamp: { generation, intentVersion: 1 } } }; },
      state: async () => ({ kind: 'producer', stamp: { generation, intentVersion: 1 } }),
    },
    producer: {
      prepare: async () => { const id = generation + 1; primed.set(id, { frames: 0, videoAtZero: false }); return { producerId: id, epoch: id }; },
      connect: async prepared => ({
        video: async time => { primed.get(prepared.producerId).videoAtZero ||= time.sampleTime === 0; calls.push(`video:${prepared.producerId}`); },
        audio: async (_time, samples) => { primed.get(prepared.producerId).frames += samples.length / 2; calls.push(`audio:${prepared.producerId}`); },
        close: () => { closed.push(prepared.producerId); },
      }),
      state: async id => ({ active: generation >= id && operationPolls > 0, acknowledged: generation < id || operationPolls > 0,
        health: 'ok', receivedFrames: primed.get(id).frames, hasVideo: primed.get(id).videoAtZero,
        epoch: id, error: null, paused: false, consumedFrames: 0,
        pendingControl: generation >= id && operationPolls === 0 ? { epoch: id, baseSample: 0, paused: false } : null }),
      ack: async control => { calls.push(`ack:${control.epoch}`); },
      release: async id => { released.push(id); },
    },
  };
  const cleanup = await activate(host);
  try {
    execute({ id: 'nes.open' });
    await waitFor(() => calls.filter(call => call === 'audio:1').length >= 4);
    assert.ok(calls.indexOf('video:1') < calls.indexOf('submit:1'));
    assert.ok(calls.indexOf('audio:1') < calls.indexOf('submit:1'));
    assert.ok(calls.indexOf('submit:1') < calls.indexOf('ack:1'));
    assert.ok(calls.indexOf('ack:1') < calls.indexOf('operation:1'));
    assert.ok(calls.includes('audio:1'));
    // Make the old pump queue a deferred stop and trigger a new open in its
    // error-report continuation, before that timer can run.
    let replacementTriggered = false;
    host.ui.set = async (_view, property, value) => {
      if (!replacementTriggered && property === 'status' && value === 'NES source replaced.') {
        replacementTriggered = true;
        execute({ id: 'nes.open' });
      }
    };
    const originalState = host.producer.state;
    host.producer.state = async id => id === 1 ? { active: false, error: null } : originalState(id);
    await waitFor(() => generation === 2 && calls.filter(call => call === 'audio:2').length >= 4);
    assert.equal(generation, 2);
    assert.ok(calls.includes('video:2'));
    assert.deepEqual(released, [1]);
    assert.deepEqual(closed, [1]);
  } finally {
    await cleanup();
    await fs.rm(directory, { recursive: true, force: true });
  }
  assert.deepEqual(released, [1, 2]);
  assert.deepEqual(closed, [1, 2]);
});
