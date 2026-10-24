import { failure, sleep, uncertain } from "./runtime.js";
import { playerOf } from "./player.js";
const TERMINAL = ["completed", "failed", "cancelled"];
/** One counter for snapshot and job operations, as the host's book expects. */
class MediaService {
    runtime;
    #operation = 0;
    constructor(runtime) {
        this.runtime = runtime;
    }
    /**
     * Run a mutation. A host reply is definite; after a lost reply the same id
     * is queried until it has an outcome (the ordered pipe means "unknown" is a
     * refusal before admission).
     */
    async mutate(send) {
        const id = ++this.#operation;
        try {
            return await send(id);
        }
        catch (error) {
            if (!uncertain(error))
                throw error;
        }
        for (;;) {
            const result = await this.runtime.host.jobs
                .operation(id)
                .catch((error) => {
                if (!uncertain(error))
                    throw error;
                return null;
            });
            if (result?.status === "completed")
                return result.result;
            if (result?.status === "failed")
                throw failure(result.error.code, result.error.message);
            if (result?.status === "unknown")
                throw failure("rejected", "media operation was not accepted");
            if (result?.status === "unknownExpired")
                throw failure("expired", "media operation outcome expired");
            await sleep(250);
        }
    }
}
const serviceOf = (runtime) => runtime.service("media", () => new MediaService(runtime));
class JobHandle {
    #scope;
    #service;
    #id;
    #state;
    #listeners = new Set();
    done;
    #disposed = false;
    constructor(scope, service, id) {
        this.#scope = scope;
        this.#service = service;
        this.#id = id;
        this.#state = {
            status: "queued",
            publicationStarted: false,
            facts: null,
            outcome: null,
        };
        scope.add(this);
        this.done = this.#poll();
        this.done.catch(() => { });
    }
    get state() {
        return this.#state;
    }
    onChange(listener) {
        this.#listeners.add(listener);
        return this.#scope.onDispose(() => {
            this.#listeners.delete(listener);
        });
    }
    #accept(job) {
        const { jobId: _, ...state } = job;
        if (JSON.stringify(state) === JSON.stringify(this.#state))
            return;
        this.#state = state;
        for (const listener of this.#listeners)
            this.#scope.run(listener, state);
        this.#scope.update();
    }
    async refresh() {
        this.#accept(await this.#service.runtime.host.jobs.state(this.#id));
        return this.#state;
    }
    async #poll() {
        while (!this.#disposed) {
            try {
                await this.refresh();
            }
            catch (error) {
                if (!uncertain(error))
                    throw error;
            }
            if (TERMINAL.includes(this.#state.status))
                return this.#state;
            await sleep(350);
        }
        throw failure("disposed", "export job handle disposed");
    }
    async cancel() {
        const result = await this.#service.mutate((operationId) => this.#service.runtime.host.jobs.cancel(this.#id, operationId));
        return result.cancel;
    }
    async dispose() {
        if (this.#disposed)
            return;
        this.#disposed = true;
        this.#scope.forget(this);
        if (!TERMINAL.includes(this.#state.status))
            await this.cancel().catch(() => { });
    }
}
class SnapshotHandle {
    #scope;
    #service;
    #frozen;
    #released = false;
    constructor(scope, service, frozen) {
        this.#scope = scope;
        this.#service = service;
        this.#frozen = frozen;
        scope.add(this);
    }
    get kind() {
        return this.#frozen.kind;
    }
    get time() {
        return this.#frozen.time;
    }
    get source() {
        return this.#frozen.source;
    }
    get duration() {
        return this.#frozen.duration;
    }
    get audioTrack() {
        return this.#frozen.audio?.trackId ?? null;
    }
    get presentation() {
        return this.#frozen.kind === "processed" ? this.#frozen.presentation : null;
    }
    get processingRevision() {
        return this.#frozen.kind === "processed"
            ? this.#frozen.processing.revision
            : null;
    }
    async export(options) {
        if (this.#released)
            throw failure("released", "snapshot already released");
        const processed = this.#frozen.kind === "processed";
        const snapshotId = this.#frozen.snapshotId;
        const mode = processed ? "processed" : "raw";
        const frame = (sampleTime) => sampleTime === undefined ? {} : { sampleTime };
        let job;
        switch (options.format) {
            case "mp4":
                job = { kind: `${mode}Mp4`, end: options.end };
                break;
            case "jpeg": {
                const quality = options.quality;
                if (quality !== undefined &&
                    !(Number.isInteger(quality) && quality >= 1 && quality <= 100))
                    throw failure("invalid", "quality must be an integer 1–100");
                job = {
                    kind: `${mode}Jpeg`,
                    ...frame(options.sampleTime),
                    ...(quality === undefined ? {} : { quality }),
                };
                break;
            }
            case "png":
            case "bmp":
                job = {
                    kind: `${mode}${options.format === "png" ? "Png" : "Bmp"}`,
                    ...frame(options.sampleTime),
                };
                break;
            default:
                throw failure("invalid", `unknown export format ${String(options.format)}`);
        }
        const result = await this.#service.mutate((operationId) => this.#service.runtime.host.jobs.start({
            operationId,
            snapshotId,
            destination: options.destination,
            ...job,
        }));
        return new JobHandle(this.#scope, this.#service, result.jobId);
    }
    async dispose() {
        if (this.#released)
            return;
        this.#released = true;
        this.#scope.forget(this);
        const snapshotId = this.#frozen.snapshotId;
        await this.#service
            .mutate((operationId) => this.#service.runtime.host.snapshot.release(snapshotId, operationId))
            .catch(() => { });
    }
}
export function mediaCapture(scope) {
    return {
        async capture(options = {}) {
            const runtime = scope.runtime;
            const service = serviceOf(runtime);
            const player = playerOf(runtime);
            const at = options.ifCurrent ?? player.state;
            const { expected } = player.expected(at);
            if (!expected)
                throw failure("noSource", "no source to capture");
            const audio = options.audio === undefined || options.audio === "none"
                ? { kind: "none" }
                : options.audio === "selected"
                    ? { kind: "selected" }
                    : { kind: "track", trackId: options.audio };
            const time = options.at === undefined ? {} : { time: options.at };
            let request;
            if (options.processed) {
                // Freeze exactly the processing/subtitle configuration of this source.
                const withSubtitles = options.subtitles !== false;
                const [processing, subtitle] = await Promise.all([
                    runtime.host.processing.state(),
                    withSubtitles
                        ? runtime.host.subtitles.state()
                        : Promise.resolve(null),
                ]);
                if (processing.source.generation !== expected.generation ||
                    (subtitle && subtitle.source.generation !== expected.generation))
                    throw failure("stale", "processing or subtitle state belongs to another source");
                request = {
                    operationId: 0,
                    expected,
                    ...time,
                    mode: "processed",
                    audio,
                    expectedProcessingRevision: processing.configurationRevision,
                    ...(subtitle
                        ? {
                            includeSubtitles: true,
                            expectedSubtitle: {
                                selection: subtitle.selection.stamp,
                                presentationRevision: subtitle.presentation.revision,
                            },
                        }
                        : { includeSubtitles: false }),
                };
            }
            else
                request = { operationId: 0, expected, ...time, mode: "raw", audio };
            const captured = await service.mutate(async (operationId) => {
                const result = await runtime.host.snapshot.capture({
                    ...request,
                    operationId,
                });
                return result;
            });
            // A recovered outcome carries only the id; read the full description.
            const frozen = "kind" in captured
                ? captured
                : await runtime.host.snapshot.state(captured.snapshotId);
            return new SnapshotHandle(scope, service, frozen);
        },
    };
}
