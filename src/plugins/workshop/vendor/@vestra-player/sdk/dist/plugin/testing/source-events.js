const inPlayback = (s) => s.source !== null && (s.phase === "loading" || s.phase === "ready");
const samePlaythrough = (a, b) => a.stamp.generation === b.stamp.generation && a.cycle === b.cycle;
/** `source.ended`'s data for a playthrough that ended in `state`. */
function endedIn(state) {
    const reason = state.terminalReason;
    if (state.phase === "error")
        return reason !== null && typeof reason === "object"
            ? { state, reason: "error", error: reason.failed }
            : { state, reason: "error" };
    return {
        state,
        reason: reason === "natural" || reason === "replaced" ? reason : "stopped",
    };
}
/** The events `next` adds to `previous`, in order. */
export function sourceEvents(previous, next, reload = false) {
    const events = [];
    const open = next.source !== null;
    const same = samePlaythrough(previous, next);
    const resumed = !reload &&
        previous.source !== null &&
        previous.stamp.generation === next.stamp.generation &&
        previous.cycle !== next.cycle &&
        previous.phase === "ended" &&
        next.phase !== "loading";
    const begins = open && !same && !resumed;
    if (inPlayback(previous) && !(same && inPlayback(next))) {
        if (same && open)
            events.push(["source.ended", endedIn(next)]);
        else if (!open && next.stamp.generation === previous.stamp.generation)
            events.push(["source.ended", { state: previous, reason: "stopped" }]);
        else
            events.push(["source.ended", { state: previous, reason: "replaced" }]);
    }
    if (begins) {
        events.push([
            "source.opening",
            {
                state: next,
                previous: previous.source !== null
                    ? { sourceId: previous.stamp.generation }
                    : null,
            },
        ]);
        if (next.phase === "ready" || next.phase === "ended")
            events.push(["source.ready", { state: next }]);
        if (next.phase === "ended" || next.phase === "error")
            events.push(["source.ended", endedIn(next)]);
    }
    else if (same && previous.phase === "loading" && open) {
        if (next.phase === "ready" || next.phase === "ended")
            events.push(["source.ready", { state: next }]);
        // Loading straight to its end: ready, then ended.
        const at = events.findIndex(([name]) => name === "source.ended");
        if (next.phase === "ended" && at >= 0)
            events.push(...events.splice(at, 1));
    }
    return events;
}
/**
 * `source.ended` with `reason: "quit"` for a source in playback (the
 * player quits; only a running plugin gets it).
 */
export function quitEvent(state) {
    return inPlayback(state) ? { state, reason: "quit" } : null;
}
