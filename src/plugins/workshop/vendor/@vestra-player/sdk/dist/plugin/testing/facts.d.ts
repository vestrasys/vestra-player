/**
 * The second batch of events in the test host (SDK 1.11.0; the player's
 * `src/host/src/fact_events.rs`): after the host's state changed, the
 * facts that changed are sent once each, by the player's rules.
 *
 * - `audio.tracks.changed`: after a `source.state`, the source's audio
 *   track list when it is known and differs from the state before.
 * - `window.changed`, `layout.changed`: after a `window.state`, when the
 *   window's state or the panel layout differ from what was last sent (the
 *   test host has no resize gesture: each change is one event).
 * - `subtitles.changed`: when a loaded source's subtitle tracks or its
 *   primary or secondary selection differ from what was last sent.
 * - `audio.devices.changed`: when the device list, the requested device or
 *   the device in use differ from what was last sent.
 * - `video.reconfigured`, `audio.reconfigured`: when a source becomes
 *   ready, from `host.stats` (its `video` and `audio`), once per source.
 *
 * The first look at each is the start, not a change. `app.started` is not
 * sent by itself: a test sends it with `host.emit("app.started", …)`.
 *
 * Internal to the SDK; not exported from `@vestra-player/sdk/testing`.
 */
import type { SourceState } from "../../source.ts";
import type { TestHost } from "./index.ts";
/** What was last sent of each fact, as JSON (`undefined`: not looked at yet). */
export interface FactsSent {
    window?: string;
    layout?: string;
    subtitles?: string;
    devices?: string;
    /** The source the reconfigurations were sent for. */
    reconfigured?: number;
    /** Set while the facts are being sent (their own emits look at nothing). */
    busy?: boolean;
}
/** `audio.tracks.changed` for the `source.state` `next` after `previous` (the player's rule). */
export declare function tracksChanged(previous: SourceState, next: SourceState): {
    sourceId: number;
    audioTracks: import("../../source.ts").SourceAudioTrack[];
} | null;
/**
 * After the host's state may have changed (a request answered, an event
 * sent): send the facts that changed. `event` is the event just sent, if
 * any.
 */
export declare function sendFacts(host: TestHost, event?: string): void;
