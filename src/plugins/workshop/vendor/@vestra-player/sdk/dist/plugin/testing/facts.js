const sent = new WeakMap();
const of = (host) => {
    let facts = sent.get(host);
    if (!facts)
        sent.set(host, (facts = {}));
    return facts;
};
/** `data` when its JSON differs from `facts[key]` (set to it); the first look sends nothing. */
function changed(facts, key, data) {
    const text = JSON.stringify(data);
    const first = facts[key] === undefined;
    if (facts[key] === text)
        return false;
    facts[key] = text;
    return !first;
}
/** `audio.tracks.changed` for the `source.state` `next` after `previous` (the player's rule). */
export function tracksChanged(previous, next) {
    if (!Array.isArray(next.audioTracks) || next.source === null)
        return null;
    if (previous.stamp.generation === next.stamp.generation &&
        JSON.stringify(previous.audioTracks) === JSON.stringify(next.audioTracks))
        return null;
    return { sourceId: next.stamp.generation, audioTracks: next.audioTracks };
}
/**
 * After the host's state may have changed (a request answered, an event
 * sent): send the facts that changed. `event` is the event just sent, if
 * any.
 */
export function sendFacts(host, event) {
    const facts = of(host);
    if (facts.busy)
        return;
    facts.busy = true;
    try {
        if (event === undefined || event === "window.state") {
            const w = host.mainWindow;
            const window = {
                fullscreen: w.fullscreen,
                maximized: w.maximized && !w.fullscreen,
                videoArea: { ...w.videoArea },
                pictureInPicture: { ...w.pictureInPicture },
            };
            if (changed(facts, "window", window))
                host.emit("window.changed", window);
            const layout = {
                ...host.layout,
                backdrop: host.backdrop && {
                    owner: 1,
                    effective: host.backdrop.effective,
                },
            };
            if (changed(facts, "layout", layout))
                host.emit("layout.changed", structuredClone(layout));
        }
        const subtitles = host.subtitles;
        if (["ready", "ended"].includes(host.source.phase)) {
            const part = {
                source: host.source.stamp.generation,
                tracks: subtitles.tracks,
                target: subtitles.selection.target,
                secondary: subtitles.selection.secondary ?? null,
            };
            // Sent for the first loaded source too, as the player does.
            const text = JSON.stringify(part);
            if (facts.subtitles !== text && (facts.subtitles = text))
                host.emit("subtitles.changed", { state: structuredClone(subtitles) });
        }
        const output = host.source.audioOutput ?? { requested: null, active: null };
        const devices = {
            devices: host.audioDevices.map(({ id, name }) => ({ id, name })),
            requested: output.requested,
            active: output.active,
        };
        if (changed(facts, "devices", devices))
            host.emit("audio.devices.changed", devices);
        const source = host.source;
        if (source.phase === "ready" &&
            facts.reconfigured !== source.stamp.generation) {
            facts.reconfigured = source.stamp.generation;
            const { video, audio } = host.stats;
            if (video?.width && video.height)
                host.emit("video.reconfigured", {
                    sourceId: source.stamp.generation,
                    width: video.width,
                    height: video.height,
                    displayWidth: video.displayWidth ?? video.width,
                    displayHeight: video.displayHeight ?? video.height,
                    fps: video.fps,
                    rotation: 0,
                    hdr: video.transfer === "pq" || video.transfer === "hlg",
                });
            if (audio?.sampleRate)
                host.emit("audio.reconfigured", {
                    sourceId: source.stamp.generation,
                    sampleRate: audio.sampleRate,
                    channels: audio.channels,
                    format: audio.format ?? null,
                });
        }
    }
    finally {
        facts.busy = false;
    }
}
