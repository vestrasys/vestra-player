                                               
import { AudioController } from './controller.js';
import { clamp, clone, neutral, presets } from './model.js';
const view = 'audio-workbench';
const callbacks = ['gain-changed', 'delay-changed', 'normalize-changed', 'clarity-changed', 'preset-selected', 'reset-requested', 'retry-requested'];
export async function activate(host      ) {
  let disposed = false;
  let opened = false;
  let opening                       = null;
  let painting = false;
  let dirty = false;
  let revision = 1;
  const ids = ['audio.workbench', 'audio.reset', ...presets.map((_, i) => `audio.preset.${i}`)];
  const controller = new AudioController(host, () => { void paint(); });
  async function paint() {
    dirty = true;
    if (painting || disposed) return;
    painting = true;
    try {
      while (dirty && !disposed) {
        dirty = false;
        const settings = clone(controller.settings);
        if (opened) {
          const values = { 'status-text': controller.status, 'source-ready': controller.ready, 'is-busy': controller.busy, 'has-error': controller.failed, 'gain-values': settings.gains, 'delay-ms': settings.delayMs, 'normalize-enabled': settings.normalize, 'clarity-db': settings.clarity };
          for (const [key, value] of Object.entries(values)) await host.ui.set(view, key, value);
        }
        const currentRevision = ++revision;
        for (const id of ids.slice(1)) await host.commands.update(id, currentRevision, { enabled: controller.ready, checked: id.startsWith('audio.preset.') ? presets[Number(id.split('.').at(-1))] .gains.every((gain, i) => gain === settings.gains[i]) : null, busy: controller.busy, reason: controller.ready ? null : '没有选中的音轨' });
      }
    } catch (error) { console.error('Audio workbench refresh failed', error); }
    finally { painting = false; }
  }
  async function show() {
    if (!opened) {
      opening ??= host.ui.open(view, 'ui/workbench.slint', {}, callbacks);
      try { await opening; opened = true; } finally { opening = null; }
    } else await host.ui.show(view, true);
    await paint();
  }
  function preset(index        ) {
    const selected = presets[index];
    if (selected) controller.edit({ ...clone(controller.settings), gains: [...selected.gains] });
  }
  for (const id of ids) {
    const index = Number(id.split('.').at(-1));
    const title = id === 'audio.workbench' ? '音频工作台…' : id === 'audio.reset' ? '复位音频处理' : presets[index] .label;
    await host.commands.register(id, 1, { title, path: id.startsWith('audio.preset.') ? ['音频', '均衡器预设', title] : ['音频', title], shortcut: null }, { enabled: id === 'audio.workbench', checked: id.startsWith('audio.preset.') ? index === 0 : null, busy: false, reason: id === 'audio.workbench' ? null : '等待音频来源' });
  }
  const offCommands = host.commands.onExecute(event => {
    if (event.id === 'audio.workbench') void show().catch(error => console.error('Audio window failed', error));
    else if (event.id === 'audio.reset') controller.edit(neutral());
    else if (event.id.startsWith('audio.preset.')) preset(Number(event.id.split('.').at(-1)));
  });
  const offUi = host.ui.onCallback(event => {
    if (event.view !== view) return;
    try {
      const settings = clone(controller.settings);
      if (event.callback === 'gain-changed') {
        const index = event.args[0];
        if (typeof index !== 'number' || !Number.isInteger(index) || index < 0 || index >= settings.gains.length) return;
        settings.gains[index] = Math.round(clamp(event.args[1], -6, 6) * 10) / 10;
      } else if (event.callback === 'delay-changed') settings.delayMs = Math.round(clamp(event.args[0], -2000, 2000) / 10) * 10;
      else if (event.callback === 'normalize-changed') settings.normalize = event.args[0] === true;
      else if (event.callback === 'clarity-changed') settings.clarity = Math.round(clamp(event.args[0], 0, 6) * 10) / 10;
      else if (event.callback === 'preset-selected') { preset(Number(event.args[0])); return; }
      else if (event.callback === 'reset-requested') { controller.edit(neutral()); return; }
      else if (event.callback === 'retry-requested') { void controller.retry(); return; }
      else return;
      controller.edit(settings);
    } catch (error) { console.error('Audio input rejected', error); }
  });
  const offSource = await host.events.subscribe('source.state', source => controller.source(source                                                ));
  controller.source(await host.source.state());
  return async () => { disposed = true; controller.dispose(); offCommands(); offUi(); await offSource(); if (opened) await host.ui.close(view); };
}
