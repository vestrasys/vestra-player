import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gate, roms, start, until } from './harness.mjs';

const pending = {
  // Producer preparation has not answered yet.
  prepare: (method) => method === 'producer.prepare',
  // The host has not answered the request that selects the stream.
  'source-submit': (method, params) => method === 'source.submit' && params.intent.kind === 'openProvider',
};

for (const [boundary, matches] of Object.entries(pending)) {
  test(`Stop while ${boundary} is pending revokes the attempt and releases its producer`, async () => {
    const files = await roms();
    const blocked = gate();
    let reached = false;
    const { host, stop, window } = await start(async (method, params, next) => {
      if (matches(method, params)) {
        reached = true;
        await blocked.promise;
      }
      return next();
    });
    try {
      host.dialogAnswers.push([files.blue]);
      void host.command('nes.open').run();
      await until(() => reached, boundary);
      window().emit('stop');
      await until(() => window().props.status === 'Stopped.', 'Stopped.');
      blocked.open();
      await until(() => host.producers.get(1)?.released, 'release');
      await host.settle(30);
      assert.equal(String(window().props.status).startsWith('Running '), false);
      assert.equal(window().props.status, 'Stopped.');
      assert.equal(window().props.running, false);
      assert.notEqual(host.source.kind === 'producer' && host.source.phase !== 'empty', true, 'the cancelled stream must not stay selected');
      const intents = host.sent('source.submit').map(p => p.intent.kind);
      if (boundary === 'prepare') assert.deepEqual(intents, [], 'a cancelled prepare must not select a source');
      else assert.deepEqual(intents, ['openProvider', 'close']);
    } finally {
      blocked.open();
      await stop();
      await files.remove();
    }
  });
}

test('a second open while one is pending is ignored', async () => {
  const files = await roms();
  const { host, stop, window } = await start();
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(() => host.windows.has('nes'), 'window');
    window().emit('open-rom'); // would need a second dialog answer if it were not ignored
    await until(() => window().props.running === true, 'ROM');
    assert.equal(host.sent('dialogs.open').length, 1);
    assert.equal(host.sent('producer.prepare').length, 1);
  } finally {
    await stop();
    await files.remove();
  }
});
