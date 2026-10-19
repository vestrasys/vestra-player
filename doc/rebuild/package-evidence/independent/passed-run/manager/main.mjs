// Independent finite consumer; loaded by the host only after a frozen-run grant.
// Fixtures and process harness are separate. This module never edits the catalog.
export async function activate(host) {
  const fs = await import('node:fs/promises');
  const path = await import('node:path');
  const {default: assert} = await import('node:assert/strict');
  const root = process.env.VESTRA_INDEPENDENT_PACKAGE_ROOT;
  const phase = process.env.VESTRA_INDEPENDENT_PACKAGE_PHASE;
  const id = 'independent.counter';
  const trace = [];
  const terminal = new Set(['committed', 'failed', 'cancelled', 'failedDegraded']);
  async function poll(label, fn, accept) {
    const until = Date.now() + 35000;
    do {
      const value = await fn();
      trace.push({label, value});
      if (accept(value)) return value;
      await new Promise(resolve => setTimeout(resolve, 30));
    } while (Date.now() < until);
    throw new Error(`Timeout: ${label}`);
  }
  const done = operation => poll(operation.operationId,
    () => host.packages.operation(operation.operationId), v => terminal.has(v.status));
  async function validate(name) {
    const value = await host.packages.validate(path.join(root, name));
    trace.push({label: `validate:${name}`, value});
    return value;
  }
  try {
    if (phase === 'lifecycle') {
      const one = await validate('one');
      const two = await validate('two');
      const bad = await validate('bad');
      const trial = await host.packages.trial(two.token);
      await poll('trial ready', () => host.packages.trialState(trial.trialId), v => v.status === 'ready');
      assert.equal((await host.packages.installed(id)).revision, 0);
      await assert.rejects(() => host.packages.install(two.token, 0, 'trial-still-active'), {code: 'busy'});
      await host.packages.stopTrial(trial.trialId);
      await poll('trial cleanup', () => host.packages.trialState(trial.trialId),
        v => v.status === 'stopped' && v.cleanupConfirmed === true);
      const first = await done(await host.packages.install(one.token, 0, 'ind-first'));
      assert.equal(first.status, 'committed');
      assert.equal(first.installed.revision, 1);
      assert.equal(first.installed.current.digest, one.digest);
      assert.equal(first.installed.previous, null);
      await assert.rejects(() => host.packages.install(two.token, 0, 'ind-stale'), {code: 'revisionConflict'});
      await assert.rejects(() => host.packages.install(two.token, 1, 'ind-first'), {code: 'conflict'});
      const failed = await done(await host.packages.install(bad.token, 1, 'ind-bad'));
      assert.equal(failed.status, 'failed');
      assert.equal(failed.restored, true);
      assert.equal((await host.packages.installed(id)).current.digest, one.digest);
      assert.equal((await host.packages.installed(id)).revision, 1);
      const second = await done(await host.packages.install(two.token, 1, 'ind-second'));
      assert.equal(second.status, 'committed');
      assert.equal(second.installed.revision, 2);
      assert.equal(second.installed.previous.digest, one.digest);
      const back = await done(await host.packages.rollback(id, 2, 'ind-back'));
      assert.equal(back.status, 'committed');
      assert.equal(back.installed.revision, 3);
      assert.equal(back.installed.current.digest, one.digest);
      assert.equal(back.installed.previous.digest, two.digest);
      assert.equal((await host.packages.cancel('ind-back')).status, 'tooLate');
      const replay = await host.packages.install(one.token, 0, 'ind-first');
      assert.equal(replay.status, 'committed');
      assert.equal(replay.installed.revision, 1);
      assert.equal((await host.packages.installed(id)).revision, 3);
      // Keep the rollback request stable across a new host process; candidate
      // validation tokens intentionally have a different, process-local scope.
      await fs.writeFile(path.join(root, 'expected.json'), JSON.stringify({one: one.digest, two: two.digest}));
    } else if (phase === 'reopen') {
      const expected = JSON.parse(await fs.readFile(path.join(root, 'expected.json'), 'utf8'));
      const installed = await host.packages.installed(id);
      assert.equal(installed.revision, 3);
      assert.equal(installed.current.digest, expected.one);
      assert.equal(installed.previous.digest, expected.two);
      const old = await host.packages.operation('ind-back');
      assert.equal(old.status, 'committed');
      assert.equal(old.installed.revision, 3);
      const replay = await host.packages.rollback(id, 2, 'ind-back');
      assert.equal(replay.status, 'committed');
      assert.equal((await host.packages.installed(id)).revision, 3);
      // Boot must execute the committed JS independently of the authoring
      // process. This observes the fixture's real activation marker.
      await poll('committed boot activation', async () => {
        try { return (await fs.readFile(path.join(root, 'activations.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse); }
        catch (error) { if (error.code === 'ENOENT') return []; throw error; }
      }, rows => rows.some(row => row.run === process.env.VESTRA_INDEPENDENT_PACKAGE_RUN && row.version === '1.0.0'));
      const commands = (await fs.readFile(path.join(root, 'commands.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse)
        .filter(row => row.run === process.env.VESTRA_INDEPENDENT_PACKAGE_RUN && row.version === '1.0.0');
      trace.push({label: 'boot public command behavior', value: commands});
      assert.deepEqual(commands.map(row => row.count), [1, 2]);
    } else {
      throw new Error(`Unknown independent phase ${phase}`);
    }
    await fs.writeFile(path.join(root, phase === 'lifecycle' ? 'lifecycle-provisional.json' : `${phase}-result.json`), JSON.stringify({passed: true, phase, trace}, null, 2));
  } catch (error) {
    await fs.writeFile(path.join(root, `${phase}-result.json`), JSON.stringify({passed: false, phase,
      error: String(error), stack: error.stack, trace}, null, 2));
  }
}
