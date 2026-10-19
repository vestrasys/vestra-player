                                               
import { clone, neutral, nodes } from './model.js';
                                           
                                                           
                                                           
                                                                    
export class AudioController {
  settings = neutral();
  ready = false;
  busy = false;
  failed = false;
  status = '请先选择音频来源';
          host                          ;
          notify            ;
          generation = 0;
          identity = '';
          epoch = 0;
          sequence = -1;
          revision = 0;
          queued                  = null;
          unresolved                = null;
          owned = false;
          disposed = false;
  constructor(host                          , notify            ) { this.host = host; this.notify = notify; }
  source(source        ) {
    if (Number.isFinite(source.eventSequence)) {
      if (source.eventSequence < this.sequence) return;
      this.sequence = source.eventSequence;
    }
    const tracks = source.audioTracks;
    const identity = `${source.stamp.generation}:${tracks?.filter(t => t.selected).map(t => t.trackId).join(',') ?? '?'}`;
    this.generation = source.stamp.generation;
    this.ready = source.phase === 'ready' && (tracks === null || tracks.some(t => t.selected));
    if (identity !== this.identity) {
      this.identity = identity;
      this.epoch++;
      this.settings = neutral();
      this.queued = this.owned ? neutral() : null;
      this.failed = this.unresolved !== null;
    }
    if (this.unresolved) this.status = '上一操作结果待确认，请检查状态';
    else if (!this.ready) this.status = source.phase === 'loading' ? '正在载入音频…' : '没有选中的音轨';
    else if (!this.busy && !this.failed) this.status = this.queued ? '正在复位当前音轨…' : '就绪';
    this.notify(); void this.pump();
  }
  edit(settings          ) {
    if (!this.ready || this.disposed) return;
    this.settings = clone(settings);
    this.queued = clone(settings);
    this.failed = this.unresolved !== null;
    this.notify(); void this.pump();
  }
  async retry() {
    if (this.disposed || this.busy || !this.ready) return;
    const flight = this.unresolved;
    if (flight) {
      this.busy = true;
      this.notify();
      try {
        const result = await this.host.processing.operation(flight.change);
        if (result.status !== 'completed' && result.status !== 'failed') {
          this.failed = true;
          this.status = '结果尚未确认，请稍后再次检查';
          return;
        }
        this.unresolved = null;
        this.failed = false;
        if (flight.epoch === this.epoch) {
          this.status = result.status === 'failed' ? result.error.message : '已应用';
          if (result.status === 'failed') this.queued ??= clone(this.settings);
        }
      } catch (error) { this.failed = true; this.status = `检查失败：${message(error)}`; }
      finally { this.busy = false; this.notify(); }
    } else { this.failed = false; this.queued ??= clone(this.settings); }
    await this.pump();
  }
          async pump() {
    if (this.disposed || this.busy || this.failed || this.unresolved || !this.ready || !this.queued) return;
    const settings = this.queued;
    this.queued = null;
    const flight         = { epoch: this.epoch, settings, change: { stage: 'audio', resourceId: 'workbench', revision: ++this.revision, expectedGeneration: this.generation, nodes: nodes(settings), offsetMs: settings.delayMs } };
    this.busy = true;
    this.owned = true;
    this.status = '正在应用…';
    this.notify();
    try {
      await this.host.processing.replace(flight.change);
      if (flight.epoch === this.epoch) this.status = '已应用';
    } catch (error) {
      try {
        const result = await this.host.processing.operation(flight.change);
        if (result.status === 'completed') { if (flight.epoch === this.epoch) this.status = '已应用'; }
        else if (result.status === 'failed') {
          // Track selection can change without advancing generation. Epoch owns all UI outcomes.
          if (flight.epoch === this.epoch) {
            this.failed = true;
            this.status = result.error.message;
            this.queued ??= clone(settings);
          }
        } else {
          // An unresolved operation is a resource barrier even after a source/track change.
          this.unresolved = flight;
          this.failed = true;
          this.status = '操作结果待确认，请检查状态后继续';
        }
      } catch { this.unresolved = flight; this.failed = true; this.status = `操作结果待确认：${message(error)}`; }
    } finally { this.busy = false; this.notify(); void this.pump(); }
  }
  dispose() { this.disposed = true; this.queued = null; }
}
function message(error         ) { return error instanceof Error ? error.message : String(error); }
