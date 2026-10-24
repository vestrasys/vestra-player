import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gate, roms, running, start, until } from './harness.mjs';

// Where a resolution can be held: the ROM was read and its producer is being
// prepared, or the prepared stream was handed to the player as the answer.
const pending = {
  prepare: (method) => method === 'producer.prepare',
  answer: (method, params) => method === 'sourceResolvers.respond' && !!params.result,
};

/** Start the plugin with the first request matching `matches` held until `blocked.open()`. */
async function held(matches) {
  const blocked = gate();
  let reached = false;
  const started = await start(async (method, params, next) => {
    if (!reached && matches(method, params)) {
      reached = true;
      await blocked.promise;
    }
    return next();
  });
  return { ...started, blocked, reached: () => reached };
}

const released = host => [...host.producers.values()].every(p => p.released);

for (const [boundary, matches] of Object.entries(pending)) {
  test(`Stop while the ${boundary} is pending closes the open and releases its producer`, async () => {
    const files = await roms();
    const { host, stop, window, blocked, reached } = await held(matches);
    try {
      host.openElsewhere(files.blue);
      await until(reached, boundary);
      assert.equal(host.source.phase, 'loading');
      await host.command('nes.stop').run();
      assert.equal(window().props.status, '已停止。');
      assert.deepEqual(host.sent('source.submit').map(p => p.intent.kind), ['close'], 'Stop closes the open in progress');
      assert.equal(host.resolutions.size, 0, 'the resolution was cancelled');
      blocked.open();
      await until(() => host.producers.size === 1 && released(host), 'release');
      await host.settle(30);
      assert.equal(window().props.status, '已停止。', 'started is never called');
      await until(() => window().props.running === false, 'running false'); // props are written one by one
      assert.equal(host.source.phase, 'empty');
    } finally {
      blocked.open();
      await stop();
      await files.remove();
    }
  });

  test(`F4 while the ${boundary} is pending: nothing starts`, async () => {
    const files = await roms();
    const { host, stop, window, blocked, reached } = await held(matches);
    try {
      host.openElsewhere(files.blue);
      await until(reached, boundary);
      await host.press('F4');
      blocked.open();
      await until(() => host.producers.size === 1 && released(host), 'release');
      await host.settle(30);
      await until(() => window().props.running === false, 'running false'); // props are written one by one
      assert.equal(String(window().props.status ?? '').startsWith('正在运行'), false);
      assert.equal(host.source.phase, 'empty');
      assert.equal(host.sent('source.submit').length, 0);
    } finally {
      blocked.open();
      await stop();
      await files.remove();
    }
  });

  test(`a newer open while the ${boundary} is pending wins; the old stream never starts`, async () => {
    const files = await roms();
    const { host, stop, window, blocked, reached } = await held(matches);
    try {
      host.openElsewhere(files.blue);
      await until(reached, boundary);
      host.openElsewhere(files.red); // e.g. playlist "next" pressed again
      await until(() => window().props.status === running('red.nes'), 'newer ROM');
      blocked.open();
      // Producer ids follow preparation order: the held one may come second.
      const live = () => [...host.producers.values()].filter(p => !p.released);
      await until(() => host.producers.size === 2 && live().length === 1, 'old producer released');
      await host.settle(30);
      assert.equal(window().props.status, running('red.nes'));
      await until(() => window().props.running === true, 'running true'); // props are written one by one
      assert.equal(host.source.source, files.red);
      assert.equal(live().length, 1);

      host.openElsewhere('/media/movie.mp4');
      await until(() => window().props.status === '已被其他媒体源替换。', 'media');
    } finally {
      blocked.open();
      await stop();
      await files.remove();
    }
  });
}

test('deactivation while a resolution is pending releases its producer', async () => {
  const files = await roms();
  const { host, stop, blocked, reached } = await held(pending.answer);
  try {
    host.openElsewhere(files.blue);
    await until(reached, 'answer');
    const stopping = stop();
    blocked.open();
    await stopping;
    await until(() => released(host), 'release');
    assert.equal(host.resolvers.size, 0);
    assert.notEqual(host.source.kind === 'producer' && host.source.phase === 'ready' && !released(host), true);
  } finally {
    blocked.open();
    await files.remove();
  }
});

test('a second picker while one is open is ignored', async () => {
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

test('a ROM picked from the menu and then replaced reports no error', async () => {
  const files = await roms();
  const { host, stop, window, blocked, reached } = await held(pending.answer);
  try {
    host.dialogAnswers.push([files.blue]);
    void host.command('nes.open').run();
    await until(reached, 'answer');
    host.openElsewhere(files.red);
    await until(() => window().props.status === running('red.nes'), 'newer ROM');
    blocked.open();
    await host.settle(700); // the menu's open learns it was replaced (operation polling)
    assert.equal(window().props.status, running('red.nes'));
  } finally {
    blocked.open();
    await stop();
    await files.remove();
  }
});
