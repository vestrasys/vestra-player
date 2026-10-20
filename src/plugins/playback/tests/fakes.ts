import type { ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import { system } from "../system.ts";

/** A stand-in for a spawned `caffeinate`: records its arguments and whether it was killed. */
export class FakeChild extends EventEmitter {
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  killed = false;
  readonly args: string[];
  constructor(args: string[]) {
    super();
    this.args = args;
  }
  kill() {
    this.killed = true;
    this.signalCode = "SIGTERM";
    queueMicrotask(() => this.emit("exit", null, "SIGTERM"));
    return true;
  }
  /** The process ended by itself (or someone killed it from outside). */
  exit(code = 0) {
    this.exitCode = code;
    this.emit("exit", code, null);
  }
}

export type Fakes = {
  children: FakeChild[];
  /** Live children (spawned, not killed, not exited). */
  readonly running: FakeChild[];
  clipboard: string;
  /** Texts written with pbcopy, newest last. */
  copied: string[];
  /** Next caffeinate start fails like a missing executable. */
  failSpawn: Error | null;
  /** Next pbcopy fails. */
  failCopy: Error | null;
};

/** Replace every system command with an in-memory fake; returns the shared record. */
export function fakeSystem(): Fakes {
  const fakes: Fakes = {
    children: [],
    get running() {
      return this.children.filter((child) => !child.killed && child.exitCode === null);
    },
    clipboard: "",
    copied: [],
    failSpawn: null,
    failCopy: null,
  };
  system.readClipboard = async () => fakes.clipboard;
  system.writeClipboard = async (text) => {
    const failure = fakes.failCopy;
    fakes.failCopy = null;
    if (failure) throw failure;
    fakes.copied.push(text);
  };
  system.caffeinate = (args) => {
    const child = new FakeChild(args);
    fakes.children.push(child);
    const failure = fakes.failSpawn;
    fakes.failSpawn = null;
    if (failure) {
      // Like spawn(): the error arrives asynchronously, and the child never ran.
      child.exitCode = -2;
      queueMicrotask(() => child.emit("error", failure));
    }
    return child as unknown as ChildProcess;
  };
  return fakes;
}
