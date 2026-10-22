import { test } from 'node:test';
import assert from 'node:assert/strict';
import { roms, start, until } from './harness.mjs';

const running = name => `正在运行 ${name} · 256×240 · 48 kHz 立体声`;

test('menu entries and window keep their paths, initial status and callbacks', async () => {
  const { host, stop, window } = await start();
  assert.deepEqual([...host.commands].map(([id, c]) => [id, c.declaration.path, c.declaration.order, c.declaration.group]), [
    ['nes.open', ['模拟器', '打开 NES ROM…'], 601, 'domain'],
    ['nes.stop', ['模拟器', '停止 NES'], 602, 'domain'],
    ['nes.window', ['模拟器', 'NES 工具…'], 603, 'domain'],
  ]);
  assert.ok([...host.commands.values()].every(c => c.state.enabled));
  await host.command('nes.window').run();
  assert.equal(window().file, 'nes.slint');
  assert.deepEqual(window().props, { status: '请选择 iNES ROM 文件。', running: false });
  assert.deepEqual(window().callbacks.sort(), ['open-rom', 'stop']);
  await stop();
});

test('initial AV prefill precedes source selection; the ROM then streams paced frames', async () => {
  const files = await roms();
  const order = [];
  const { host, stop, window } = await start(async (method, params, next) => {
    if (method === 'source.submit' && params.intent.kind === 'openProvider') {
      const producer = host.producers.get(Number(params.intent.producerId));
      order.push({ submit: producer.receivedFrames, video: producer.videoFrames });
    }
    if (method === 'producer.ack') order.push('ack');
    return next();
  });
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => window().props.status === running('blue.nes') && window().props.running === true, 'running');
    assert.equal(host.sent('dialogs.open')[0].title, '打开 NES ROM');
    const [submit, ack] = order;
    assert.ok(submit.submit >= 1024 && submit.video > 0, 'selection needs buffered PCM and a frame');
    assert.equal(ack, 'ack', 'the start control is acknowledged after selection');
    assert.equal(host.source.kind, 'producer');
    const producer = host.producers.get(1);
    const before = producer.receivedFrames;
    await until(() => producer.receivedFrames > before + 4800, 'paced frames');
    assert.equal(producer.released, false);
  } finally {
    await stop();
    await files.remove();
  }
  assert.equal(host.producers.get(1).released, true, 'deactivation releases the stream');
});

test('replacement by another source shows the old wording; a newer ROM is never torn down by the old one', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => window().props.running === true, 'first ROM');
    host.openElsewhere('/movie.mp4');
    await until(() => window().props.status === '已被其他媒体源替换。', 'replacement');
    assert.equal(window().props.running, false);
    assert.equal(host.producers.get(1).released, true);
    assert.equal(host.sent('source.submit').some(p => p.intent.kind === 'close'), false, 'never closes a source it no longer owns');
    assert.equal(host.source.source, '/movie.mp4');

    host.dialogAnswers.push([files.red]);
    window().emit('open-rom');
    await until(() => window().props.status === running('red.nes'), 'second ROM');
    await host.settle(30);
    assert.equal(host.producers.get(2).released, false);
    assert.equal(window().props.running, true);
    assert.equal(host.source.source, 'producer://2');
  } finally {
    await stop();
    await files.remove();
  }
});

test('opening another ROM replaces our own stream; a bad ROM keeps the running game', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => window().props.running === true, 'first ROM');

    host.dialogAnswers.push([files.bad]);
    await host.command('nes.open').run();
    assert.equal(window().props.status, '错误：请选择 iNES 格式的 .nes ROM 文件。');
    assert.equal(window().props.running, true);
    assert.equal(host.producers.get(1).released, false);

    host.dialogAnswers.push(null); // cancelled picker changes nothing
    await host.command('nes.open').run();
    assert.equal(window().props.status, '错误：请选择 iNES 格式的 .nes ROM 文件。');

    host.dialogAnswers.push([files.red]);
    void host.command('nes.open').run();
    await until(() => window().props.status === running('red.nes'), 'second ROM');
    assert.equal(host.producers.get(1).released, true);
    assert.equal(host.source.source, 'producer://2');
  } finally {
    await stop();
    await files.remove();
  }
});

// The real host fails a selected producer source whose transport closes
// ("producer transport closed", shown as 打开失败 by the playback plugin):
// the source must already be closed when the plugin drops the connection.
test('Stop (window button, menu) and a newer ROM close the source before the producer transport', async () => {
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
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => window().props.running === true, 'first ROM');
    host.dialogAnswers.push([files.red]);
    void host.command('nes.open').run();
    await until(() => window().props.status === running('red.nes'), 'second ROM');
    await until(() => closedUnder.length === 1, 'first transport closed');
    assert.equal(closedUnder[0], 'null empty', 'the first stream was closed before its transport');

    window().emit('stop');
    await until(() => window().props.status === '已停止。', 'stopped');
    await until(() => closedUnder.length === 2, 'second transport closed');
    assert.equal(closedUnder[1], 'null empty', 'closed before its transport');
    assert.equal(host.source.terminalReason, 'stopped');
    assert.equal(host.sent('source.submit').filter(p => p.intent.kind === 'close').length, 2);
    assert.equal(host.producers.get(2).released, true);
  } finally {
    await stop();
    await files.remove();
  }
});

test('Stop closes our own source, releases it and says so', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => window().props.running === true, 'ROM');
    await host.command('nes.stop').run();
    assert.equal(window().props.status, '已停止。');
    assert.equal(window().props.running, false);
    assert.equal(host.producers.get(1).released, true);
    assert.equal(host.sent('source.submit').at(-1).intent.kind, 'close');
    assert.equal(host.source.phase, 'empty');
    window().emit('stop'); // nothing running: still reports 已停止。
    await host.settle();
    assert.equal(window().props.status, '已停止。');
  } finally {
    await stop();
    await files.remove();
  }
});

test('start-up and stream failures keep the plugin wording', async () => {
  const files = await roms();
  let health = 'deviceLost';
  let lost = null;
  const { host, stop, window } = await start(async (method, params, next) => {
    const result = await next();
    if (method !== 'producer.state') return result;
    return { ...result, health: result.active ? result.health : health, error: lost };
  });
  try {
    host.dialogAnswers.push([files.blue]);
    await until(() => host.commands.size === 3);
    void host.command('nes.open').run();
    await until(() => window().props.status === '错误：音频设备未就绪。', 'device failure');
    assert.equal(host.producers.get(1).released, true);
    assert.equal(host.sent('source.submit').length, 0);

    health = 'ok';
    host.dialogAnswers.push([files.blue]);
    window().emit('open-rom');
    await until(() => window().props.running === true, 'ROM');
    lost = 'audio output lost';
    await until(() => window().props.status === '模拟器已停止：audio output lost', 'stream error');
    assert.equal(window().props.running, false);
    assert.equal(host.producers.get(2).released, true);
  } finally {
    await stop();
    await files.remove();
  }
});
