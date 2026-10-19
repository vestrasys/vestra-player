// Orchestration doubles only. No native/media rendering assertions.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { activate } from '../dist/main.js';
const wait = ms => new Promise(r => setTimeout(r, ms));
function fixture() {
  const calls = [], ui = new Map(), events = new Map();
  let callback, command;
  let state = {
    source: { generation: 1, phase: 'ready' }, confirmation: 'applied', error: null,
    selection: { stamp: { generation: 1, revision: 1 }, target: { kind: 'embedded', id: 1 } },
    tracks: [{ target: { kind: 'embedded', id: 1 }, title: 'English', language: 'en', styleSupported: true, styleReason: null }], resources: [],
    presentation: { revision: 1, referenceHeight: 1080, layers: [], effective: null, confirmation: 'applied', fonts: [{ id: 'fixed', family: 'Fixed' }], actualFont: {}, snapshotEligible: true, coverageCheckedOnCapture: true },
  };
  const copy = () => structuredClone(state);
  const forbidden = new Proxy({}, { get: () => () => { throw new Error('Must not use playback/source controls'); } });
  const host = {
    playback: forbidden, source: forbidden,
    commands: { register: async (...args) => { calls.push(['register', ...args]); }, update: async () => {}, onExecute: fn => { command = fn; return () => {}; } },
    events: { subscribe: async (name, fn) => { events.set(name, fn); return () => events.delete(name); } },
    ui: { onCallback: fn => { callback = fn; return () => { callback = undefined; }; }, open: async (...args) => { calls.push(['ui.open', ...args]); }, show: async () => {}, set: async (_id, name, value) => { ui.set(name, value); } },
    dialogs: { open: async () => ({ cancelled: true, paths: [] }) },
    subtitles: {
      state: async () => copy(),
      select: async (target, stamp, operationId) => { calls.push(['select', target, stamp, operationId]); state.selection = { target, stamp: { ...stamp, revision: stamp.revision + 1 } }; return copy(); },
      style: async options => { calls.push(['style', options]); state.presentation.revision++; state.presentation.effective = options.style && { owner: 4, manifestId: 'vestra.subtitles', revision: options.revision, style: options.style }; return copy(); },
      begin: async (...args) => { calls.push(['begin', ...args]); }, append: async (...args) => { calls.push(['append', ...args]); },
      commit: async options => { calls.push(['commit', options]); return copy(); },
      discard: async (...args) => { calls.push(['discard', ...args]); }, operation: async operationId => { calls.push(['operation', operationId]); return { status: 'unknown' }; },
    },
  };
  return { host, calls, ui, events, fire: (name, value) => callback?.({ view: 'subtitle-workbench', callback: name, args: value === undefined ? [] : [value] }), command: id => command({ id, revision: 1 }), get: copy, set: value => { state = value; } };
}
async function ready(f) { f.command('workbench'); await wait(30); assert.ok(f.calls.find(c => c[0] === 'ui.open')); }
test('independent Window uses callbacks only and complete Chinese menu leaf paths', async () => {
  const f = fixture(), stop = await activate(f.host);
  try {
    await ready(f);
    const window = f.calls.find(c => c[0] === 'ui.open');
    assert.equal(window[2], 'ui/workbench.slint'); assert.ok(!window[4].includes('busy'));
    for (const c of f.calls.filter(c => c[0] === 'register')) assert.deepEqual(c[3].path, ['字幕', c[3].title]);
    f.fire('change-delay', 200); f.fire('change-delay', 650); f.fire('change-size', 52); await wait(170);
    const styles = f.calls.filter(c => c[0] === 'style'); assert.equal(styles.length, 1);
    assert.equal(styles[0][1].style.offsetMs, 650); assert.equal(styles[0][1].style.fontSize, 52);
  } finally { await stop(); }
});
test('file picker cancellation never begins upload', async () => {
  const f = fixture(), stop = await activate(f.host);
  try { await ready(f); f.fire('load-file'); await wait(30); assert.ok(!f.calls.some(c => c[0] === 'begin')); assert.match(f.ui.get('status-text'), /已取消选择/); }
  finally { await stop(); }
});
test('source change fences a picker before file access', async () => {
  const f = fixture(); let resolvePicker;
  f.host.dialogs.open = () => new Promise(resolve => { resolvePicker = resolve; });
  const stop = await activate(f.host);
  try {
    await ready(f); f.fire('load-file'); await wait(20);
    const next = f.get(); next.source.generation = 2; next.selection.stamp.generation = 2; f.set(next);
    f.events.get('source.state')({ stamp: { generation: 2 }, phase: 'ready' });
    resolvePicker({ cancelled: false, paths: ['/must-not-be-read.srt'] }); await wait(30);
    assert.ok(!f.calls.some(c => c[0] === 'begin')); assert.match(f.ui.get('status-text'), /媒体已切换/);
  } finally { await stop(); }
});
test('old track list cannot select matching embedded id in new source', async () => {
  const f = fixture(), stop = await activate(f.host);
  try { await ready(f); const next = f.get(); next.source.generation = 2; next.selection.stamp.generation = 2; f.set(next); f.fire('select-track', 1); await wait(30); assert.ok(!f.calls.some(c => c[0] === 'select')); }
  finally { await stop(); }
});
test('source replacement and disposal cancel pending drag writes', async () => {
  const f = fixture(), stop = await activate(f.host);
  await ready(f); f.fire('change-delay', 900);
  const next = f.get(); next.source.generation = 2; next.selection.stamp.generation = 2; f.set(next);
  f.events.get('source.state')({ stamp: { generation: 2 }, phase: 'ready' }); await wait(140);
  assert.ok(!f.calls.some(c => c[0] === 'style')); f.fire('change-size', 40); await stop(); await wait(140);
  assert.ok(!f.calls.some(c => c[0] === 'style'));
});
test('real file bytes are chunked and hashed; unknown commit is queried and never discarded', async () => {
  const directory = await mkdtemp(fileURLToPath(new URL('./fixture-', import.meta.url)));
  const path = directory + '/actual.srt'; const bytes = Buffer.from('1\n00:00:00,000 --> 00:00:01,000\nHello\n'); await writeFile(path, bytes);
  const f = fixture(); f.host.dialogs.open = async () => ({ cancelled: false, paths: [path] });
  f.host.subtitles.commit = async options => { f.calls.push(['commit', options]); throw Object.assign(new Error('timeout'), { code: 'timeout' }); };
  const stop = await activate(f.host);
  try {
    await ready(f); f.fire('load-file'); await wait(60);
    assert.equal(f.calls.filter(c => c[0] === 'begin').length, 1);
    const appended = f.calls.filter(c => c[0] === 'append'); assert.equal(Buffer.from(appended[0][3], 'base64').toString(), bytes.toString());
    const commits = f.calls.filter(c => c[0] === 'commit'); assert.equal(commits.length, 1);
    assert.match(commits[0][1].sha256, /^[a-f0-9]{64}$/); assert.deepEqual(commits[0][1].expected, { generation: 1, revision: 1 });
    assert.equal(f.calls.find(c => c[0] === 'operation')[1], commits[0][1].operationId);
    assert.ok(!f.calls.some(c => c[0] === 'discard')); assert.match(f.ui.get('status-text'), /结果尚未确认/);
  } finally { await stop(); await rm(directory, { recursive: true, force: true }); }
});
