/**
 * Plugin-generated audio/video (host "producer"). The SDK owns preroll,
 * activation, pause/epoch acknowledgements, input acknowledgements and pacing;
 * the plugin only renders frames.
 */
import { failure, sleep, toError } from "./runtime.js";
import { abortError } from "./player.js";
/** Frames of audio buffered ahead of the device before rendering pauses. */
const AHEAD = 2400;
/** Frames written before the host is asked to select the stream. */
const PREROLL = 2048;
class StreamHandle {
    #running = true;
    #stopping;
    #listeners = new Set();
    #scope;
    #service;
    #producer;
    #socket;
    #generation;
    #frame;
    #epoch;
    #sample;
    #sequence;
    #timer;
    #pumping = Promise.resolve();
    constructor(scope, service, setup) {
        this.#scope = scope;
        this.#service = service;
        this.#producer = setup.producer;
        this.#socket = setup.socket;
        this.#generation = setup.generation;
        this.#frame = setup.frame;
        this.#epoch = setup.epoch;
        this.#sample = setup.sample;
        this.#sequence = setup.sequence;
        scope.add(this);
        this.#pumping = this.#pump();
    }
    get running() {
        return this.#running;
    }
    onStop(listener) {
        this.#listeners.add(listener);
        return this.#scope.onDispose(() => {
            this.#listeners.delete(listener);
        });
    }
    async #pump() {
        const producer = this.#service.runtime.host.producer;
        try {
            const state = await producer.state(this.#producer);
            if (!this.#running)
                return;
            if (!state.active || state.error) {
                void this.#end(state.error ? failure("stream", state.error) : null);
                return;
            }
            // Controls apply at a frame boundary: no frame renders concurrently.
            const control = state.pendingControl;
            if (control) {
                this.#epoch = control.epoch;
                this.#sample = control.baseSample;
                this.#sequence = 0;
                await producer.ack(control);
            }
            if (state.input?.pending)
                await producer.ackInput(state.input.pending);
            const paused = control ? control.paused : state.paused;
            if (!control &&
                state.acknowledged &&
                state.epoch === this.#epoch &&
                !paused &&
                this.#sample - state.consumedFrames < AHEAD)
                await this.#render();
            if (this.#running)
                this.#timer = setTimeout(() => {
                    this.#pumping = this.#pump();
                }, 5);
        }
        catch (error) {
            void this.#end(toError(error));
        }
    }
    async #render() {
        const frame = this.#frame();
        await writeFrame(this.#socket, frame, this, this.#epoch);
    }
    /** Shared by preroll and the pump: audio sample time defines the AV timeline. */
    advance(samples) {
        this.#sample += samples;
    }
    get sample() {
        return this.#sample;
    }
    nextSequence() {
        return this.#sequence++;
    }
    async #end(error) {
        if (!this.#running)
            return;
        await this.stop();
        for (const listener of this.#listeners)
            this.#scope.run(listener, error);
    }
    /**
     * Close the source first (if it is still ours), while the frame connection
     * is still up: a connection that drops under a selected source is a
     * playback failure to the player ("producer transport closed"). Then the
     * connection, then the producer.
     */
    stop() {
        this.#stopping ??= (async () => {
            this.#running = false;
            clearTimeout(this.#timer);
            await this.#pumping.catch(() => { });
            try {
                await this.#service.closeIfCurrent(this.#generation);
            }
            finally {
                this.#socket.close();
                await this.#service.runtime.host.producer
                    .release(this.#producer)
                    .catch(() => { });
            }
        })();
        return this.#stopping;
    }
    async dispose() {
        this.#scope.forget(this);
        await this.stop().catch(() => { });
    }
}
/** Start-up timeouts say which step ran out of time. */
function timeout(stage, message) {
    return Object.assign(failure("timeout", message), { stage });
}
async function writeFrame(socket, frame, clock, epoch) {
    try {
        await sendFrame(socket, frame, clock, epoch);
    }
    catch (error) {
        // The frame broke a limit (size, sample range): the plugin's mistake.
        if (error instanceof TypeError)
            throw failure("invalid", error.message);
        throw error;
    }
}
async function sendFrame(socket, frame, clock, epoch) {
    await socket.video({ epoch, sampleTime: clock.sample, sequence: clock.nextSequence() }, frame.video);
    for (let offset = 0; offset < frame.audio.length; offset += 9600) {
        const samples = frame.audio.subarray(offset, offset + 9600);
        await socket.audio({ epoch, sampleTime: clock.sample, sequence: clock.nextSequence() }, samples);
        clock.advance(samples.length / 2);
    }
}
/**
 * Prepare a producer, connect and preroll it until the host could select it.
 * `check` throws to give up (an abort); the producer is then released.
 */
async function prepare(host, options, check) {
    const bytes = options.width * options.height * 4;
    /** The plugin's frame, checked against the declared size. */
    const frame = () => {
        const next = options.frame();
        if (next?.video?.byteLength !== bytes)
            throw failure("invalid", `frame().video must be ${options.width}×${options.height}×4 = ${bytes} bytes of RGBA`);
        return next;
    };
    let producer;
    let socket;
    try {
        check();
        const spec = await host.producer.prepare({
            width: options.width,
            height: options.height,
        });
        producer = spec.producerId;
        check();
        socket = await host.producer.connect(spec);
        check();
        const p = {
            producer,
            socket,
            frame,
            epoch: spec.epoch,
            sample: 0,
            sequence: 0,
        };
        const clock = clockOf(p);
        // A prepared producer is paused at sample zero. Supply a frame at zero and
        // enough contiguous PCM before asking the host to select it.
        while (p.sample < PREROLL) {
            check();
            await writeFrame(socket, frame(), clock, p.epoch);
        }
        for (const deadline = Date.now() + 10_000;; await sleep(10)) {
            check();
            const state = await host.producer.state(producer);
            if (state.error)
                throw failure("stream", state.error);
            if (state.epoch !== p.epoch)
                throw failure("stream", "producer epoch changed before activation");
            if (!["ok", "recoverableXrun", "deviceChanged"].includes(state.health))
                throw failure("device", `audio device is not ready: ${state.health}`);
            if (state.acknowledged && state.receivedFrames >= 1024 && state.hasVideo)
                return p;
            if (Date.now() >= deadline)
                throw timeout("prefill", "timed out preparing the first buffers");
        }
    }
    catch (error) {
        socket?.close();
        if (producer !== undefined)
            await host.producer.release(producer).catch(() => { });
        throw error;
    }
}
const clockOf = (p) => ({
    get sample() {
        return p.sample;
    },
    advance: (n) => (p.sample += n),
    nextSequence: () => p.sequence++,
});
/**
 * Wait until the host runs the selected producer: acknowledge its start
 * control. `selected` says whether the host has accepted the selection yet
 * (`"selected"`: it has, and a producer no longer active was replaced);
 * `check` throws to give up.
 */
async function activate(host, p, check, selected) {
    for (const deadline = Date.now() + 10_000;; await sleep(10)) {
        check();
        const state = await host.producer.state(p.producer);
        if (state.error)
            throw failure("stream", state.error);
        if (selected === "selected" && !state.active)
            throw abortError();
        if (state.pendingControl) {
            p.epoch = state.pendingControl.epoch;
            p.sample = state.pendingControl.baseSample;
            p.sequence = 0;
            await host.producer.ack(state.pendingControl);
        }
        if ((selected === "selected" || selected()) &&
            state.active &&
            !state.pendingControl &&
            state.acknowledged &&
            state.epoch === p.epoch)
            return;
        if (Date.now() >= deadline)
            throw timeout("activation", "timed out waiting for stream activation");
    }
}
/** Give a prepared producer up: its source (if still ours), connection, producer. */
async function abandon(service, p, generation) {
    // A cancelled open may already have selected this producer: close only
    // that source, before its frame connection (as in `stop`).
    if (generation !== undefined)
        await service.closeIfCurrent(generation).catch(() => { });
    p.socket.close();
    await service.runtime.host.producer.release(p.producer).catch(() => { });
}
const handleOf = (scope, service, p, generation) => new StreamHandle(scope, service, {
    producer: p.producer,
    socket: p.socket,
    generation,
    frame: p.frame,
    epoch: p.epoch,
    sample: p.sample,
    sequence: p.sequence,
});
export async function openStream(scope, service, options) {
    const host = service.runtime.host;
    const check = () => {
        if (options.signal?.aborted || scope.disposed)
            throw abortError();
    };
    const p = await prepare(host, options, check);
    let generation;
    /** The request selecting the stream. */
    let opened;
    try {
        let accepted = false;
        let failed;
        opened = service
            .submit({ kind: "openProvider", producerId: p.producer, startPaused: false }, { accepted: (ticket) => (generation = ticket.stamp.generation) })
            .then((ticket) => {
            generation = ticket.stamp.generation;
            accepted = true;
        });
        opened.catch((error) => (failed = error));
        // Activation needs this side to acknowledge the host's start control.
        await activate(host, p, () => {
            check();
            if (failed)
                throw failed;
        }, () => accepted);
        return handleOf(scope, service, p, generation);
    }
    catch (error) {
        // An abort can arrive before the host answered the selecting request: wait
        // for that answer so a source selected afterwards is still closed.
        await opened?.catch(() => { });
        await abandon(service, p, generation);
        throw error;
    }
}
/**
 * A file resolver's stream (`ResolvedStream`): prepared and prerolled while
 * the open waits, then handed to the host as the answer. Once the host
 * selected it, the start control is acknowledged and `started` gets the
 * running stream, synchronously, before any `onStop`. Cancelled before or
 * during activation (the open was replaced or closed, the plugin stopped),
 * the producer is released and `started` is never called.
 */
export async function resolvedStream(scope, service, stream, signal) {
    const host = service.runtime.host;
    const p = await prepare(host, stream, () => {
        if (signal.aborted || scope.disposed)
            throw abortError();
    });
    return {
        kind: "stream",
        producerId: p.producer,
        abandoned: () => abandon(service, p, undefined),
        accepted: async ({ stamp }) => {
            try {
                await activate(host, p, () => {
                    if (scope.disposed)
                        throw abortError();
                }, "selected");
                // Replaced before it ran: the source is no longer this stream.
                if (service.raw.stamp.generation > stamp.generation)
                    throw abortError();
            }
            catch (error) {
                await abandon(service, p, stamp.generation);
                if (error.code !== "aborted")
                    scope.report(error);
                return;
            }
            const handle = handleOf(scope, service, p, stamp.generation);
            if (stream.started)
                scope.run(stream.started, handle);
        },
    };
}
