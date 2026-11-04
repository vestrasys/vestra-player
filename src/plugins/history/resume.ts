import type { JsonStore, Osd, Player, PlayerState, Plugin, Window } from "@vestra-player/sdk";
import { clock, fileSize, keyOf, legacyKeyOf, mediaKind } from "./identity.ts";
import { quiet } from "./menu.ts";
import { jsonStore, timing } from "./store.ts";

export type Mode = "off" | "auto" | "ask";
export type Memo = { position: number; duration: number; size: number | null; updatedAt: number };
type Saved = { version: 1; mode: Mode; audio: boolean; items: Record<string, Memo> };

export const CAPACITY = 1000;
/** Shorter sources are not remembered. */
export const MIN_DURATION = 60;
/** Earlier positions count as "from the start". */
export const MIN_POSITION = 10;
/** Within this many seconds of the end (or past 95 %) the source counts as finished. */
export const END_MARGIN = 15;
/** Resuming goes back this far, to pick the story up again. */
export const REWIND = 2;

/** The source being watched: its identity and the last facts seen for it. */
type Track = {
  sourceId: number;
  source: string;
  key: string;
  /** The other key 0.1.3 stored for this source, if any (`legacyKeyOf`); its memo moves to `key`. */
  legacy: string | null;
  size: number | null;
  /** `null` until known (extension table, then `player.stats()`). */
  audio: boolean | null;
  position: number | null;
  duration: number | null;
  paused: boolean;
  /** The first ready state of this source; the condition of the resume seek. */
  ready: PlayerState | null;
  /** The resume decision is still open: the position is not recorded meanwhile. */
  holding: boolean;
  /** The resume decision has been started (or was not wanted). */
  decided: boolean;
  /** Played to the end; recording starts again on a replay. */
  ended: boolean;
};

const finished = (position: number, duration: number) =>
  position > duration - END_MARGIN || position >= duration * 0.95;

/**
 * B25 resume playback: remembers where a media source was left and, when it is
 * opened again from anywhere, continues there (or asks first), in
 * `<dataDirectory>/positions.json`.
 */
export class Resume {
  mode: Mode = "auto";
  audio = false;
  items = new Map<string, Memo>();
  readonly #player: Player;
  readonly #osd: Osd;
  readonly #update: () => void;
  readonly #store: JsonStore<Saved>;
  readonly #resumeWindow: Window;
  readonly #listeners: (() => void)[] = [];
  #track: Track | null = null;
  #ask: { track: Track; memo: Memo; timer: ReturnType<typeof setTimeout> } | null = null;
  /** The remembered point the ask window shows (`12:34`). */
  #position = "";

  constructor(plugin: Plugin, osd: Osd) {
    this.#player = plugin.player;
    this.#osd = osd;
    this.#update = () => plugin.update();
    // Created before the onDispose below, so it is released after it and writes the last position.
    this.#store = jsonStore<Saved>(plugin, "positions", { version: 1, mode: "auto", audio: false, items: {} }, timing.positionsWrite);
    const saved = this.#store.value as { mode?: unknown; audio?: unknown; items?: unknown };
    if (saved.mode === "off" || saved.mode === "auto" || saved.mode === "ask") this.mode = saved.mode;
    if (typeof saved.audio === "boolean") this.audio = saved.audio;
    if (saved.items && typeof saved.items === "object")
      Object.entries(saved.items as Record<string, Memo>)
        .filter(
          ([, memo]) =>
            !!memo &&
            Number.isFinite(memo.position) &&
            Number.isFinite(memo.duration) &&
            (memo.size === null || Number.isFinite(memo.size)) &&
            Number.isFinite(memo.updatedAt),
        )
        .sort(([, a], [, b]) => a.updatedAt - b.updatedAt) // oldest first: the LRU order
        .slice(-CAPACITY)
        .forEach(([key, memo]) => this.items.set(key, memo));

    this.#resumeWindow = plugin.windows.create("ui/resume.slint");
    this.#resumeWindow.bind(() => ({ position: this.#position }));
    this.#resumeWindow.on("resume", () => this.#answer(true));
    this.#resumeWindow.on("restart", () => this.#answer(false));

    const tick = setInterval(() => {
      const track = this.#track;
      if (track && !track.paused && this.#player.state.phase === "ready") this.#record(track, false);
    }, timing.positionTick);
    plugin.onDispose(() => {
      clearInterval(tick);
      this.#closeAsk();
      if (this.#track) this.#record(this.#track, true); // the store, released next, writes it
    });
  }

  /** Called when a remembered position appears, moves at a pause or a source change, or disappears. */
  onChange(listener: () => void) {
    this.#listeners.push(listener);
  }

  /** The remembered position for a record, if any (the A12 menu shows it); also one saved under the 0.1.3 key. */
  memo({ key, source }: { key: string; source: string }): Memo | undefined {
    const legacy = legacyKeyOf(source);
    return this.items.get(key) ?? (legacy === null ? undefined : this.items.get(legacy));
  }

  setMode(mode: Mode) {
    if (mode === this.mode) return;
    this.mode = mode;
    if (mode !== "ask") this.#closeAsk();
    this.#save();
  }

  setAudio(audio: boolean) {
    if (audio === this.audio) return;
    this.audio = audio;
    this.#save();
  }

  clear() {
    this.items.clear();
    this.#save();
    this.#changed();
  }

  /**
   * Every player state. `initial` is the state at activation: a source that is
   * already playing is tracked, but not resumed (the plugin was only reloaded).
   */
  observe(state: PlayerState, initial = false) {
    let track = this.#track;
    if (track && (track.sourceId !== state.sourceId || state.kind !== "media" || !state.source)) {
      // Replaced or closed: remember where it was last seen (unless it played to the end).
      if (!track.ended) this.#record(track, true);
      if (this.#ask?.track === track) this.#closeAsk();
      this.#track = track = null;
    }
    if (state.kind !== "media" || !state.source || state.phase === "empty" || state.phase === "error") return;
    if (!track) {
      const kind = mediaKind(state.source);
      track = this.#track = {
        sourceId: state.sourceId,
        source: state.source,
        key: keyOf(state.source),
        legacy: legacyKeyOf(state.source),
        size: fileSize(state.source),
        audio: kind === null ? null : kind === "audio",
        position: null,
        duration: null,
        paused: state.paused,
        ready: null,
        holding: false,
        decided: initial && state.phase !== "loading",
        ended: false,
      };
    }
    const wasPaused = track.paused;
    if (state.position !== null) track.position = state.position;
    if (state.duration !== null) track.duration = state.duration;
    track.paused = state.paused;

    if (state.phase === "ended") {
      if (!track.ended) {
        track.ended = true;
        if (state.endReason === "natural") this.#forget(track);
        else this.#record(track, true);
      }
      return;
    }
    if (state.phase !== "ready") return;
    if (track.ended) track.ended = false; // replayed (e.g. a single-item loop): no resume, recording again

    if (!track.ready) {
      track.ready = state;
      if (!track.decided) track.holding = true;
    }
    if (!track.decided && track.duration !== null) {
      track.decided = true;
      void this.#resume(track);
    } else if (state.paused && !wasPaused) this.#record(track, true);
  }

  /** Whether B25 applies to this source; resolves `audio` with `player.stats()` when the extension says nothing. */
  async #applies(track: Track): Promise<boolean> {
    if (track.duration === null || track.duration < MIN_DURATION) return false;
    if (track.audio === null) {
      try {
        track.audio = (await this.#player.stats()).video === null;
      } catch {
        return false; // unknown for now; asked again at the next decision
      }
    }
    return !track.audio || this.audio;
  }

  async #resume(track: Track) {
    try {
      if (!(await this.#applies(track)) || this.mode === "off") return;
      const memo = this.#adopt(track);
      if (!memo || this.#track !== track) return;
      if (memo.size !== track.size || memo.position > track.duration! - END_MARGIN) {
        // The file was replaced or cut shorter: the point no longer means anything.
        this.#forget(track);
        return;
      }
      // Seeked, paused or replaced since it became ready: their change stands.
      if (!this.#player.isCurrent(track.ready!)) return;
      if (this.mode === "ask") {
        this.#openAsk(track, memo);
        return;
      }
      await this.#seek(track, memo);
    } finally {
      if (!this.#ask || this.#ask.track !== track) track.holding = false;
    }
  }

  /**
   * The memo of `track`; one saved under its 0.1.3 key moves to the current key
   * (same LRU place). 0.1.3 saw no file there, so it saved no size: the size
   * seen now is taken as the file's.
   */
  #adopt(track: Track): Memo | undefined {
    const memo = this.items.get(track.key);
    const legacy = track.legacy;
    if (memo || legacy === null) return memo;
    const saved = this.items.get(legacy);
    if (!saved) return undefined;
    const old = saved.size === null ? { ...saved, size: track.size } : saved;
    this.items = new Map([...this.items].map(([key, value]) => (key === legacy ? [track.key, old] : [key, value])));
    this.#save();
    this.#changed();
    return old;
  }

  async #seek(track: Track, memo: Memo) {
    try {
      await this.#player.seek(Math.max(0, memo.position - REWIND), { ifCurrent: track.ready! });
      quiet(this.#osd.show("Resuming from {}", [clock(memo.position)]));
    } catch (error) {
      if ((error as { code?: string }).code !== "stale")
        quiet(this.#osd.show("Cannot resume: {}", [error instanceof Error ? error.message : String(error)]));
    }
  }

  #openAsk(track: Track, memo: Memo) {
    this.#closeAsk();
    this.#position = clock(memo.position);
    const timer = setTimeout(() => this.#answer(false), timing.askTimeout);
    this.#ask = { track, memo, timer };
    this.#update();
    void this.#resumeWindow.show();
  }

  #answer(resume: boolean) {
    const ask = this.#ask;
    if (!ask) return;
    this.#closeAsk();
    ask.track.holding = false;
    if (resume && this.#track === ask.track) void this.#seek(ask.track, ask.memo);
  }

  #closeAsk() {
    const ask = this.#ask;
    if (!ask) return;
    clearTimeout(ask.timer);
    this.#ask = null;
    ask.track.holding = false;
    void this.#resumeWindow.hide().catch(() => {});
  }

  /** Remember (or, near the start or the end, forget) where `track` is. `settled`: a pause, a change of source, the end. */
  #record(track: Track, settled: boolean) {
    if (this.mode === "off" || track.holding || track.position === null) return;
    const { duration, position } = track;
    if (duration === null || duration < MIN_DURATION || track.audio === null || (track.audio && !this.audio)) return;
    if (position < MIN_POSITION || finished(position, duration)) {
      this.#forget(track);
      return;
    }
    const before = this.items.get(track.key);
    this.items.delete(track.key); // insertion order is the LRU order
    if (track.legacy !== null) this.items.delete(track.legacy); // replaced by the current key
    this.items.set(track.key, { position, duration, size: track.size, updatedAt: Date.now() });
    while (this.items.size > CAPACITY) this.items.delete(this.items.keys().next().value!);
    if (before?.position !== position) this.#save();
    if (settled || !before) this.#changed();
  }

  #forget(track: Track) {
    if (this.mode === "off") return;
    const legacy = track.legacy !== null && this.items.delete(track.legacy);
    if (!this.items.delete(track.key) && !legacy) return;
    this.#save();
    this.#changed();
  }

  #save() {
    this.#store.save({ version: 1, mode: this.mode, audio: this.audio, items: Object.fromEntries(this.items) });
  }

  #changed() {
    for (const listener of this.#listeners) listener();
  }
}
