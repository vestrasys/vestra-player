import type { Host } from '../../sdk/index.ts';
import { readFile } from 'node:fs/promises';
import { basename } from 'node:path';
import { NesCore } from './core.js';

type Producer = Awaited<ReturnType<Host['producer']['connect']>>;
type Active = { id: number; core: NesCore; socket: Producer; epoch: number; sample: number; sequence: number; generation: number; cancelled: boolean };

export async function activate(host: Host) {
  let active: Active | undefined;
  let disposed = false;
  let windowOpen = false;
  let opening = false;
  let openingAttempt = 0;
  let tick: ReturnType<typeof setTimeout> | undefined;
  let pumpTask: Promise<void> = Promise.resolve();
  let operationId = 0;
  let status = 'Choose an iNES ROM to start.';
  const view = 'nes';
  async function report(message: string) {
    status = message;
    if (windowOpen) {
      await host.ui.set(view, 'status', status);
      await host.ui.set(view, 'running', !!active);
    }
  }
  async function show() {
    if (disposed) return;
    if (windowOpen) return host.ui.show(view, true);
    await host.ui.open(view, 'nes.slint', { status, running: !!active }, ['open-rom', 'stop']);
    windowOpen = true;
  }
  async function stop(expected?: Active, cancelPending = expected === undefined) {
    if (expected && active !== expected) return;
    if (cancelPending) openingAttempt++;
    if (tick) clearTimeout(tick);
    const previous = active;
    active = undefined;
    if (!previous) return;
    previous.cancelled = true;
    await pumpTask.catch(() => {});
    previous.socket.close();
    try {
      const source = await host.source.state();
      if (source.kind === 'producer' && source.stamp.generation === previous.generation)
        await host.source.submit({ kind: 'close', expected: source.stamp }, ++operationId);
    } finally {
      await host.producer.release(previous.id);
    }
  }
  async function pump(session: Active) {
    if (session.cancelled || disposed || active !== session) return;
    try {
      const state = await host.producer.state(session.id);
      if (session.cancelled) return;
      if (!state.active || state.error) {
        await report(state.error ?? 'NES source replaced.');
        // Do not await this task from inside itself.
        setTimeout(() => { void stop(session).catch(error => report(String(error))); }, 0);
        return;
      }
      const control = state.pendingControl;
      if (control) {
        // No core frame runs concurrently: each control is observed at a frame boundary.
        session.epoch = control.epoch;
        session.sample = control.baseSample;
        session.sequence = 0;
        await host.producer.ack(control);
      }
      if (state.input?.pending) await host.producer.ackInput(state.input.pending);
      const paused = control ? control.paused : state.paused;
      if (!control && state.acknowledged && state.epoch === session.epoch && !paused && session.sample - state.consumedFrames < 2400) {
        const frame = session.core.frame();
        const time = { epoch: session.epoch, sampleTime: session.sample, sequence: session.sequence++ };
        await session.socket.video(time, frame.video);
        // Audio sample timestamps, rather than JS timers, define the AV timeline.
        for (let offset = 0; offset < frame.audio.length; offset += 9600) {
          const samples = frame.audio.subarray(offset, offset + 9600);
          await session.socket.audio({ epoch: session.epoch, sampleTime: session.sample, sequence: session.sequence++ }, samples);
          session.sample += samples.length / 2;
        }
      }
      if (!session.cancelled && active === session)
        tick = setTimeout(() => { pumpTask = pump(session); }, 5);
    } catch (error) {
      await report(`NES stopped: ${String(error)}`);
      setTimeout(() => { void stop(session).catch(() => {}); }, 0);
    }
  }
  async function openRom() {
    if (opening || disposed) return;
    opening = true;
    const attempt = ++openingAttempt;
    const checkOpening = () => {
      if (disposed || attempt !== openingAttempt) throw new Error('NES open cancelled');
    };
    let preparedId: number | undefined;
    let sourceGeneration: number | undefined;
    let socket: Producer | undefined;
    try {
      await show();
      checkOpening();
      const result = await host.dialogs.open({ kind: 'openFile', title: 'Open NES ROM' });
      checkOpening();
      if (result.cancelled || !result.paths[0]) return;
      const path = result.paths[0];
      const bytes = await readFile(path);
      checkOpening();
      if (bytes.byteLength > 16 * 1024 * 1024) throw new Error('ROM exceeds the 16 MiB limit');
      const core = new NesCore(bytes);
      await stop(active, false);
      checkOpening();
      const prepared = await host.producer.prepare({ width: 256, height: 240 });
      preparedId = prepared.producerId;
      checkOpening();
      socket = await host.producer.connect(prepared);
      checkOpening();
      let epoch = prepared.epoch;
      let sample = 0;
      let sequence = 0;
      // A prepared producer is paused at sample zero. Supply a frame at zero
      // and enough contiguous PCM before asking the host to select it.
      while (sample < 2048) {
        checkOpening();
        const frame = core.frame();
        await socket.video({ epoch, sampleTime: sample, sequence: sequence++ }, frame.video);
        checkOpening();
        for (let offset = 0; offset < frame.audio.length; offset += 9600) {
          const samples = frame.audio.subarray(offset, offset + 9600);
          await socket.audio({ epoch, sampleTime: sample, sequence: sequence++ }, samples);
          checkOpening();
          sample += samples.length / 2;
        }
      }
      const initialDeadline = Date.now() + 10000;
      while (true) {
        checkOpening();
        const state = await host.producer.state(prepared.producerId);
        checkOpening();
        if (state.error) throw new Error(state.error);
        if (state.epoch !== epoch) throw new Error('Producer epoch changed before activation');
        if (!['ok', 'recoverableXrun', 'deviceChanged'].includes(state.health))
          throw new Error(`Audio device is not ready: ${state.health}`);
        if (state.acknowledged && state.receivedFrames >= 1024 && state.hasVideo) break;
        if (Date.now() >= initialDeadline) throw new Error('Timed out preparing first NES AV buffers');
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      const id = ++operationId;
      let operation = await host.source.submit({ kind: 'openProvider', producerId: prepared.producerId, startPaused: false }, id);
      if (operation.status === 'failed') throw new Error(`Cannot open NES source: ${JSON.stringify(operation.error)}`);
      sourceGeneration = operation.ticket.stamp.generation;
      checkOpening();
      const deadline = Date.now() + 10000;
      while (true) {
        checkOpening();
        const state = await host.producer.state(prepared.producerId);
        checkOpening();
        if (state.error) throw new Error(state.error);
        if (state.pendingControl) {
          epoch = state.pendingControl.epoch;
          sample = state.pendingControl.baseSample;
          sequence = 0;
          await host.producer.ack(state.pendingControl);
        }
        if (operation.status === 'pending') operation = await host.source.operation(id);
        checkOpening();
        if (operation.status === 'failed') throw new Error(`NES source failed: ${JSON.stringify(operation.error)}`);
        if (operation.status === 'completed' && state.active && !state.pendingControl && state.acknowledged && state.epoch === epoch) break;
        if (Date.now() >= deadline) throw new Error('Timed out waiting for NES source activation');
        await new Promise(resolve => setTimeout(resolve, 10));
      }
      const session: Active = { id: prepared.producerId, core, socket, epoch, sample, sequence, generation: operation.ticket.stamp.generation, cancelled: false };
      active = session;
      preparedId = undefined;
      socket = undefined;
      await report(`Running ${basename(path)} · 256 × 240 · stereo 48 kHz`);
      pumpTask = pump(session);
    } catch (error) {
      socket?.close();
      if (sourceGeneration !== undefined && preparedId !== undefined) {
        // A cancelled in-flight open may already have selected this producer.
        // Close only that generation; never close a replacement source.
        try {
          const current = await host.source.state();
          if (current.kind === 'producer' && current.stamp.generation === sourceGeneration)
            await host.source.submit({ kind: 'close', expected: current.stamp }, ++operationId);
        } catch { /* Producer owner cleanup is the final release fence. */ }
      }
      if (preparedId !== undefined) await host.producer.release(preparedId).catch(() => {});
      if (!disposed && attempt === openingAttempt) await report(String(error));
    } finally { opening = false; }
  }
  const offCommand = host.commands.onExecute(event => {
    if (event.id === 'nes.open') void openRom();
    if (event.id === 'nes.window') void show().catch(error => report(String(error)));
    if (event.id === 'nes.stop') void stop().then(() => report('Stopped.')).catch(error => report(String(error)));
  });
  const offCallback = host.ui.onCallback(event => {
    if (event.view !== view) return;
    if (event.callback === 'open-rom') void openRom();
    if (event.callback === 'stop') void stop().then(() => report('Stopped.')).catch(error => report(String(error)));
  });
  const state = { enabled: true, checked: null, busy: false, reason: null };
  await host.commands.register('nes.open', 1, { title: '打开 NES ROM…', path: ['模拟器', '打开 NES ROM…'], shortcut: null }, state);
  await host.commands.register('nes.window', 1, { title: 'NES 工具', path: ['模拟器', 'NES 工具'], shortcut: null }, state);
  await host.commands.register('nes.stop', 1, { title: '停止 NES', path: ['模拟器', '停止 NES'], shortcut: null }, state);
  return async () => {
    disposed = true;
    offCommand();
    offCallback();
    await stop();
    if (windowOpen) await host.ui.close(view);
  };
}
