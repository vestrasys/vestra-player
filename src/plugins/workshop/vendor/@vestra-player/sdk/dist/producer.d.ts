import type { SourceStamp, SourceTicket } from "./source.ts";
import type { Transport } from "./wire.ts";
export interface PreparedProducer {
    producerId: number;
    /** A Unix socket path, or `tcp://127.0.0.1:<port>` (loopback, with `token`). */
    socket: string;
    /** Hex bytes to send before any packet (loopback TCP only). */
    token?: string;
    epoch: number;
    sampleRate: 48000;
    channels: 2;
    sampleFormat: "f32le";
    videoFormat: "rgba8";
    maxAudioFrames: 4800;
    audioCapacityFrames: 9600;
    videoCapacity: 8;
}
export type ProducerControl = {
    producerId: number;
    stamp: SourceStamp;
    cycle: number;
    epoch: number;
    baseSample: number;
    paused: boolean;
};
export type ProducerInputScope = {
    owner: number;
    producerId: number;
    ticket: SourceTicket;
    inputEpoch: number;
};
export type ProducerInputNotice = {
    scope: ProducerInputScope;
    sequence: number;
    pressed: string[];
    captured: boolean;
    reason: "focusLost" | "windowHidden" | "menu" | "dialog" | "pause" | "sourceChanged" | "providerGone" | "disabled" | "reconfigured" | "transportFailure" | null;
};
export type ProducerInputState = {
    state: ProducerInputNotice;
    pending: ProducerInputNotice | null;
    keys: string[];
};
export interface ProducerState {
    input: ProducerInputState | null;
    pendingControl: ProducerControl | null;
    producerId: number;
    epoch: number;
    acknowledged: boolean;
    paused: boolean;
    consumedFrames: number;
    underrunFrames: number;
    deviceErrors: number;
    health: "ok" | "recoverableXrun" | "deviceChanged" | "streamInvalidated" | "deviceUnavailable" | "otherError";
    xrunEvents: number;
    deviceChangeEvents: number;
    errorDetails: string[];
    errorOverflow: number;
    buffer: {
        device: string;
        supported: string;
        defaultConfig: string;
        requestedFrames: number;
        actualFrames: number;
        oneBufferMilliseconds: number;
    };
    receivedFrames: number;
    hasVideo: boolean;
    error: string | null;
    active: boolean;
}
export interface PacketTime {
    epoch: number;
    sampleTime: number;
    sequence: number;
}
export declare function producerClient(transport: Transport): {
    /** Provider promises its core has frozen/resumed at a frame boundary; echo is not the audio callback ACK. */
    ack: (control: ProducerControl) => Promise<{
        status: "acknowledged";
        control: ProducerControl;
    }>;
    /** Physical identifiers remain distinct; the provider maps them to its own controls. */
    declareInput: (producerId: number, keys: string[]) => Promise<ProducerInputState>;
    /** Apply the complete set at the core boundary before echoing this notice. */
    ackInput: (notice: ProducerInputNotice) => Promise<{
        status: "acknowledged";
        sequence: number;
    }>;
    prepare: (spec: {
        width: number;
        height: number;
    }) => Promise<PreparedProducer>;
    state: (producerId: number) => Promise<ProducerState>;
    release: (producerId: number) => Promise<void>;
    connect(prepared: PreparedProducer): Promise<{
        audio(time: PacketTime, samples: Float32Array): Promise<void>;
        video(time: PacketTime, rgba: Uint8Array): Promise<void>;
        close(): void;
    }>;
};
/** The host's transport address: loopback TCP or a Unix socket path. */
export declare function connectTo(prepared: Pick<PreparedProducer, "socket">): import("node:net").Socket;
