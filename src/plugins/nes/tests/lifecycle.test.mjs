import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TestHost } from '@vestra-player/sdk/testing';
import { roms, running, start, until } from './harness.mjs';

/** The plugin's stream is the player's source, under the path that was opened. */
/** The real player retires a replaced stream's producer; the TestHost alone keeps it. */
const mode = retire => (retire ? 'producer retired like the player' : 'TestHost producer kept');

const playing = (host, path) =>
  host.source.kind === 'producer' && host.source.source === path && host.source.phase === 'ready';

test('menu entries, window, file resolver and drop provider are registered', async () => {
  const { host, stop, window } = await start();
  assert.deepEqual([...host.commands].map(([id, c]) => [id, c.declaration.path, c.declaration.order, c.declaration.group]), [
    ['nes.open', ['模拟器', '打开 NES ROM…'], 601, 'domain'],
    ['nes.stop', ['模拟器', '停止 NES'], 602, 'domain'],
    ['nes.window', ['模拟器', 'NES 工具…'], 603, 'domain'],
  ]);
  assert.ok([...host.commands.values()].every(c => c.state.enabled));
  assert.deepEqual(host.resolvers.get('nes'), { id: 'nes', extensions: ['nes'] });
  assert.deepEqual(host.dropProviders.map(p => p.extensions), [['nes']]);
  await host.command('nes.window').run();
  assert.equal(window().file, 'nes.slint');
  assert.deepEqual(window().props, { status: '请选择 iNES ROM 文件。', running: false });
  assert.deepEqual(window().callbacks.sort(), ['open-rom', 'stop']);
  await stop();
  assert.equal(host.resolvers.size, 0, 'deactivation unregisters the resolver: .nes opens as a plain file again');
  assert.equal(host.dropProviders.length, 0);
});

test('a .nes opened elsewhere (⌘O, playlist) runs as a stream under its path; prefill precedes the answer', async () => {
  const files = await roms();
  const order = [];
  const { host, stop, window } = await start(async (method, params, next) => {
    if (method === 'sourceResolvers.respond' && params.result) {
      const producer = host.producers.get(Number(params.result.producerId));
      order.push({ frames: producer.receivedFrames, video: producer.videoFrames });
    }
    if (method === 'producer.ack') order.push('ack');
    return next();
  });
  try {
    host.openElsewhere(files.blue);
    await until(() => window().props.status === running('blue.nes') && window().props.running === true, 'running');
    assert.equal(playing(host, files.blue), true);
    assert.deepEqual(host.sent('dialogs.open'), [], 'no picker for a file opened elsewhere');
    const [answer, ack] = order;
    assert.ok(answer.frames >= 1024 && answer.video > 0, 'the answer carries buffered PCM and a frame');
    assert.equal(ack, 'ack', 'the start control is acknowledged after selection');
    const producer = host.producers.get(1);
    const before = producer.receivedFrames;
    await until(() => producer.receivedFrames > before + 4800, 'paced frames');
    assert.equal(producer.released, false);
    assert.equal(host.resolutions.size, 0);

    host.openElsewhere(files.upper); // the extension matches in any case
    await until(() => window().props.status === running('GAME.NES'), 'upper-case extension');
    assert.equal(playing(host, files.upper), true);
  } finally {
    await stop();
    await files.remove();
  }
  assert.ok([...host.producers.values()].every(p => p.released), 'deactivation releases the streams');
});

test('the menu entry and the window button pick a ROM and open it like any file', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    host.dialogAnswers.push([files.blue]);
    await host.command('nes.open').run();
    await until(() => window().props.status === running('blue.nes'), 'menu ROM');
    assert.equal(host.sent('dialogs.open')[0].title, '打开 NES ROM');
    assert.deepEqual(host.sent('source.submit').map(p => [p.intent.kind, p.intent.source]), [['open', files.blue]]);
    assert.equal(playing(host, files.blue), true);

    host.dialogAnswers.push(null); // a cancelled picker changes nothing
    window().emit('open-rom');
    await until(() => host.sent('dialogs.open').length === 2, 'second picker');
    await host.settle(30); // the event reaches the plugin later than settle() would see
    assert.equal(window().props.status, running('blue.nes'));

    host.dialogAnswers.push([files.red]);
    window().emit('open-rom');
    await until(() => window().props.status === running('red.nes'), 'button ROM');
    assert.equal(playing(host, files.red), true);
    await until(() => host.producers.get(1).released, 'the first stream released');
  } finally {
    await stop();
    await files.remove();
  }
});

test('dropping files on the video area opens the first ROM; other files are not ours', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    await host.drop([files.movie, files.red, files.blue], { target: 'video' });
    await until(() => window().props.status === running('red.nes'), 'dropped ROM');
    assert.equal(playing(host, files.red), true);
    assert.deepEqual(host.sent('source.submit').map(p => p.intent.source), [files.red]);
  } finally {
    await stop();
    await files.remove();
  }
});

for (const retire of [true, false]) {
  test(`replacement by another source shows the old wording; a newer ROM is never torn down by the old one (${mode(retire)})`, async () => {
    const files = await roms();
    const { host, stop, window } = await start(undefined, undefined, { retire });
    try {
      host.openElsewhere(files.blue);
      await until(() => window().props.running === true, 'first ROM');
      host.openElsewhere('/media/movie.mp4');
      await until(() => window().props.status === '已被其他媒体源替换。', 'replacement');
      await until(() => host.producers.get(1).released, 'first stream released');
      await until(() => window().props.running === false, 'running false'); // props are written one by one
      assert.equal(host.sent('source.submit').some(p => p.intent.kind === 'close'), false, 'never closes a source it no longer owns');
      assert.equal(host.source.source, '/media/movie.mp4');

      host.openElsewhere(files.red);
      await until(() => window().props.status === running('red.nes'), 'second ROM');
      await host.settle(30);
      assert.equal(host.producers.get(2).released, false);
      await until(() => window().props.running === true, 'running true'); // props are written one by one
      assert.equal(playing(host, files.red), true);
    } finally {
      await stop();
      await files.remove();
    }
  });
}

// Playlist next / previous are `player.open(path)` by the playlist plugin:
// the same as an open elsewhere, through a list mixing media and ROMs.
for (const retire of [true, false]) {
  test(`next / previous through a mixed list: ROM → ROM → media → ROM (${mode(retire)})`, async () => {
    const files = await roms();
    const { host, stop, window } = await start(undefined, undefined, { retire });
    try {
      host.openElsewhere(files.blue);
      await until(() => window().props.status === running('blue.nes'), 'blue');
      host.openElsewhere(files.red); // ROM straight after ROM
      await until(() => window().props.status === running('red.nes') && playing(host, files.red), 'red');
      await until(() => host.producers.get(1).released, 'blue released');
      await host.settle(30);
      assert.equal(window().props.status, running('red.nes'), 'the old stream stopping does not overwrite the new status');
      await until(() => window().props.running === true, 'running true'); // props are written one by one
      host.openElsewhere('/media/movie.mp4');
      await until(() => window().props.status === '已被其他媒体源替换。', 'media');
      host.openElsewhere(files.blue);
      await until(() => window().props.status === running('blue.nes') && playing(host, files.blue), 'blue again');
      assert.equal(host.producers.get(3).released, false);
      assert.equal(host.sent('source.submit').length, 0, 'the plugin itself never submitted anything');
    } finally {
      await stop();
      await files.remove();
    }
  });
}

for (const retire of [true, false]) {
  test(`closing the source (F4) stops the game and says 已停止。 (${mode(retire)})`, async () => {
    const files = await roms();
    const { host, stop, window } = await start(undefined, undefined, { retire });
    try {
      host.openElsewhere(files.blue);
      await until(() => window().props.running === true, 'ROM');
      await host.press('F4');
      await until(() => window().props.status === '已停止。', 'stopped by F4');
      await until(() => window().props.running === false, 'running false'); // props are written one by one
      await until(() => host.producers.get(1).released, 'released');
      assert.equal(host.source.phase, 'empty');
    } finally {
      await stop();
      await files.remove();
    }
  });
}

for (const retire of [true, false]) {
  test(`a bad ROM fails the open with the plugin wording, in the player and the window (${mode(retire)})`, async () => {
    const files = await roms();
    const { host, stop, window } = await start(undefined, undefined, { retire });
    const cases = [
      [files.bad, '这不是 iNES 格式的 NES ROM 文件。'],
      [files.truncated, 'ROM 文件不完整：比文件头声明的短。'],
      [files.large, 'ROM 文件超过 16 MiB 上限。'],
      [files.missing, '找不到 ROM 文件。'],
    ];
    try {
      host.openElsewhere(files.blue);
      await until(() => window().props.running === true, 'ROM');
      for (const [path, message] of cases) {
        host.openElsewhere(path);
        await until(() => host.source.phase === 'error', `error for ${path}`);
        assert.equal(host.source.source, path);
        assert.deepEqual(host.source.terminalReason, { failed: message }, 'the player shows the plugin text, unprefixed');
        await until(() => window().props.status === `错误：${message}`, `status for ${path}`);
        await until(() => window().props.running === false, 'running false'); // props are written one by one
      }
      assert.equal(host.producers.size, 1, 'a bad ROM never prepares a stream');
      assert.equal(host.producers.get(1).released, true, 'the open replaced the running game first');

      host.dialogAnswers.push([files.bad]); // the menu entry: the same failure
      await host.command('nes.open').run();
      await until(() => host.source.phase === 'error' && window().props.status === '错误：这不是 iNES 格式的 NES ROM 文件。', 'menu');
    } finally {
      await stop();
      await files.remove();
    }
  });
}

// Real player (GUI, 2026-10-24): test.nes running, ⌘O bad.nes showed
// 「模拟器已停止：producer absent」: the old stream's stop overwrote the failure.
for (const retire of [true, false]) {
  test(`a running ROM replaced by a ROM that fails to resolve keeps the failure text (${mode(retire)})`, async () => {
    const files = await roms();
    const { host, stop, window } = await start(undefined, undefined, { retire });
    try {
      host.openElsewhere(files.blue);
      await until(() => window().props.running === true, 'ROM');
      host.openElsewhere(files.bad);
      await until(() => host.source.phase === 'error', 'failed open');
      await until(() => host.producers.get(1).released, 'old stream released');
      await host.settle(100); // the old stream's stop has been handled
      assert.equal(window().props.status, '错误：这不是 iNES 格式的 NES ROM 文件。');
      assert.equal(window().props.running, false);
      assert.equal(host.source.source, files.bad);
      assert.equal(host.sent('source.submit').length, 0, 'nothing closed or reopened by the plugin');
    } finally {
      await stop();
      await files.remove();
    }
  });
}

// The real host fails a selected producer source whose transport closes
// ("producer transport closed", shown as 打开失败 by the playback plugin):
// the source must already be closed when the plugin drops the connection.
test('Stop (window button, menu) closes the source before the producer transport', async () => {
  const files = await roms();
  const closedUnder = [];
  const { host, stop, window } = await start(async (method, params, next) => {
    const result = await next();
    if (method === 'producer.prepare') {
      host.producers.get(result.producerId).server.on('connection', socket =>
        socket.on('close', () => closedUnder.push(`${host.source.source} ${host.source.phase}`)));
    }
    return result;
  });
  try {
    host.openElsewhere(files.blue);
    await until(() => window().props.running === true, 'ROM');
    window().emit('stop');
    await until(() => window().props.status === '已停止。', 'stopped');
    await until(() => closedUnder.length === 1, 'transport closed');
    assert.equal(closedUnder[0], 'null empty', 'closed before its transport');
    assert.equal(host.source.terminalReason, 'stopped');
    assert.deepEqual(host.sent('source.submit').map(p => p.intent.kind), ['close']);
    assert.equal(host.producers.get(1).released, true);

    host.openElsewhere(files.red);
    await until(() => window().props.running === true, 'second ROM');
    await host.command('nes.stop').run();
    assert.equal(window().props.status, '已停止。');
    await until(() => window().props.running === false, 'running false'); // props are written one by one
    await until(() => closedUnder.length === 2, 'second transport closed');
    assert.equal(closedUnder[1], 'null empty');
    assert.equal(host.source.phase, 'empty');

    window().emit('stop'); // nothing running: still reports 已停止。 and submits nothing
    await host.settle(30);
    assert.equal(window().props.status, '已停止。');
    assert.equal(host.sent('source.submit').length, 2);
  } finally {
    await stop();
    await files.remove();
  }
});

test('stream failures keep the plugin wording', async () => {
  const files = await roms();
  let health = 'deviceLost';
  let lost = null;
  const { host, stop, window } = await start(async (method, params, next) => {
    const result = await next();
    if (method !== 'producer.state') return result;
    return { ...result, health: result.active ? result.health : health, error: lost };
  });
  try {
    // Preparation fails after resolve returned: the open fails with the SDK's
    // Chinese start-up text (SDK 1.5.7+), shown by the player and the window alike.
    host.openElsewhere(files.blue);
    await until(() => host.source.phase === 'error', 'device failure');
    assert.deepEqual(host.source.terminalReason, { failed: '音频输出设备不可用' });
    await until(() => window().props.status === '错误：音频输出设备不可用', 'device status');
    await until(() => host.producers.get(1).released, 'released');

    health = 'ok';
    host.openElsewhere(files.blue);
    await until(() => window().props.running === true, 'ROM');
    lost = 'audio output lost';
    await until(() => window().props.status === '模拟器已停止：audio output lost', 'stream error');
    await until(() => window().props.running === false, 'running false'); // props are written one by one
    await until(() => host.producers.get(2).released, 'released');
  } finally {
    await stop();
    await files.remove();
  }
});

test('another plugin holding .nes: activation goes on, the menu still opens files', async () => {
  const files = await roms();
  const host = new TestHost();
  host.otherResolvers.push({ owner: 'example.nes', extensions: ['nes'] });
  const { stop, window } = await start(undefined, host);
  try {
    assert.equal(host.resolvers.size, 0);
    assert.equal(host.commands.size, 3);
    host.dialogAnswers.push([files.blue]);
    await host.command('nes.open').run();
    await host.settle(30);
    assert.equal(host.source.kind, 'media', 'opened as a plain file: the other resolver would take it');
    assert.equal(host.producers.size, 0);
    await until(() => window().props.running === false, 'running false'); // props are written one by one
  } finally {
    await stop();
    await files.remove();
  }
});
