// The rows of one conversation as the window shows them, built from the
// worker's live events or from pi's JSONL. One user turn is: the user's
// message, ONE activity row (while the model works: "Working…" and the latest
// tool call; after the turn: "Worked for 12 seconds · 3 steps", expandable to
// the tool calls), then the assistant's answer.
//
// The model's thinking is not shown: it only keeps a turn's activity row (a
// turn that thought and called no tool still says how long it worked). The
// answer is the text of model messages that did not end in a tool call; the
// text of one that did is a progress note and is not shown either (while it
// streams it shows as the answer; the record keeps it). A turn that ends
// without an answer shows its last note as the answer.
//
// Rows carry data, not sentences: notices, errors and the activity row say
// what happened with a `code` and its `args`; ai.slint has the words (@tr).
import type { WorkerEvent } from "./worker/protocol.ts";

/** One tool call (a step): its tool, what it is about, 0 running / 1 done / 2 failed. */
export type ToolCall = {
  tool: string;
  target: string;
  /** `target` is a file of the read-only reference (shown as such). */
  reference: boolean;
  state: number;
};

const NO_CALL: ToolCall = { tool: "", target: "", reference: false, state: 0 };

export interface Row {
  id: string;
  /** "user" | "activity" | "assistant" | "notice" | "error" */
  kind: string;
  /** user and assistant: the message (data). */
  text: string;
  /** error: the provider's message. */
  detail: string;
  /** 0 running, 1 done, 2 failed */
  state: number;
  expanded: boolean;
  /** error rows: offer Open Settings. */
  settings: boolean;
  /**
   * notice / error: what to say (see ai.slint). activity: "working" (the
   * turn runs; `call` is the latest step, if any), "done" (the turn is over:
   * `seconds`, `n` steps, `failed` failed).
   */
  code: string;
  args: string[];
  /** notice: the count of a plural notice; activity: the number of steps. */
  n: number;
  /** activity: how long the turn worked, in seconds (0: from the record, which has no durations). */
  seconds: number;
  /** activity: every step of the turn, and the latest one. */
  calls: ToolCall[];
  call: ToolCall;
  failed: number;
}

/**
 * A provider or network failure: the error row's `code` (and the model it
 * names), and whether the key is the likely cause.
 */
export function classify(message: string, model = ""): { code: string; args: string[]; settings: boolean } {
  // The endpoint refuses the reasoning parameters (thinking, reasoning_effort, budget_tokens…).
  if (
    /(reasoning|thinking|budget_tokens|enable_thinking)/i.test(message) &&
    /not supported|unsupported|does not support|doesn't support|not allowed|not permitted|unknown|unrecognized|unexpected|invalid|extra (inputs|fields)/i.test(message)
  )
    return { code: "thinking", args: [], settings: true };
  if (/\b(401|403)\b|invalid.{0,20}(api.?key|x-api-key|token)|incorrect api key|authentication|authenticate|unauthori[sz]ed/i.test(message))
    return { code: "key", args: [], settings: true };
  if (
    /model.{0,60}(not found|no longer available|not available|does not exist|not exist|is not supported|unsupported|deprecated|retired)|(unknown|invalid|unsupported) model|model_not_found/i.test(message)
  )
    return model ? { code: "model", args: [model], settings: true } : { code: "model-unnamed", args: [], settings: true };
  if (/\b429\b|quota|insufficient|credit|billing|rate.?limit/i.test(message)) return { code: "quota", args: [], settings: false };
  if (/ECONNREFUSED|ENOTFOUND|EAI_AGAIN|ECONNRESET|ETIMEDOUT|fetch failed|network|socket|getaddrinfo|Connection error/i.test(message))
    return { code: "network", args: [], settings: false };
  return { code: "failed", args: [], settings: false };
}

let serial = 0;

const nextId = () => `r${++serial}`;

export class Chat {
  rows: Row[] = [];
  private calls: (ToolCall & { id: string })[] = [];
  private activityRow: Row | undefined;
  private assistant: Row | undefined;
  /** A live turn is in progress (the activity row says "Working…"). */
  private live = false;
  /** When the live turn began (its duration is the activity row's). */
  private startedAt = 0;
  /** The model thought in this turn (keeps the activity row of a turn without steps). */
  private thought = false;
  /** The turn's latest progress note: the answer after all if the turn ends without one. */
  private lastNote = "";
  /** Where the current model message's text starts in the answer row (-1: none open). */
  private messageStart = -1;
  private readonly now: () => number;
  constructor(now: () => number = Date.now) {
    this.now = now;
  }
  /** The text of the last user message, for Retry. */
  lastPrompt = "";
  /** The model of the current run, named in a "model not available" error. */
  model = "";

  private static row(fields: Pick<Row, "kind"> & Partial<Row>): Row {
    return { id: nextId(), text: "", detail: "", state: 1, expanded: false, settings: false, code: "", args: [], n: 0, seconds: 0, calls: [], call: NO_CALL, failed: 0, ...fields };
  }

  private push(fields: Pick<Row, "kind"> & Partial<Row>): Row {
    const full = Chat.row(fields);
    this.rows.push(full);
    return full;
  }

  user(text: string) {
    this.endTurn();
    this.lastPrompt = text;
    this.push({ kind: "user", text });
  }

  /**
   * A live turn starts (a message was sent): the activity row appears at once
   * as "Working…", before the model's first event, and follows every event.
   */
  begin() {
    this.live = true;
    this.startedAt = this.now();
    this.activity();
    this.refreshActivity();
  }

  private activity(): Row {
    if (!this.activityRow) {
      const row = Chat.row({ kind: "activity", state: 0 });
      // Before the turn's text: the row reads as "what the agent did".
      const at = this.assistant ? this.rows.indexOf(this.assistant) : -1;
      if (at >= 0) this.rows.splice(at, 0, row);
      else this.rows.push(row);
      this.activityRow = row;
    }
    return this.activityRow;
  }

  /** A notice row: `code` and its arguments (see ai.slint). */
  notice(code: string, args: (string | number)[] = []) {
    this.push({ kind: "notice", code, args: args.map(String) });
  }

  /** An error row for the provider's `message`; `code` says it instead of the classification. */
  error(message: string, code?: string, args: (string | number)[] = []) {
    const found = classify(message, this.model);
    this.push({
      kind: "error",
      code: code ?? found.code,
      args: code ? args.map(String) : found.args,
      detail: message.split("\n")[0].slice(0, 400),
      state: 2,
      settings: code ? false : found.settings,
    });
  }

  private refreshActivity() {
    const row = this.activityRow;
    if (!row) return;
    const failed = this.calls.filter((call) => call.state === 2).length;
    const plain = ({ tool, target, reference, state }: ToolCall): ToolCall => ({ tool, target, reference, state });
    if (this.live) {
      // While the turn runs: "Working…" and the latest step (each new one replaces the previous one).
      const last = this.calls.at(-1);
      row.state = 0;
      row.code = "working";
      row.call = last ? plain(last) : NO_CALL;
    } else {
      row.state = failed ? 2 : 1;
      row.code = "done";
      row.call = NO_CALL;
    }
    row.n = this.calls.length;
    row.failed = failed;
    row.calls = this.calls.map(plain);
  }

  private toolCall(id: string, call: Omit<ToolCall, "state">, state: number) {
    this.activity();
    const known = this.calls.find((c) => c.id === id);
    if (known) known.state = state;
    else this.calls.push({ id, ...call, state });
    this.refreshActivity();
  }

  /** Thinking of the model: not shown, only counted as work. */
  private thinking(delta: string) {
    if (delta) this.thought = true;
  }

  private appendText(text: string) {
    if (!text) return;
    if (!this.assistant) this.assistant = this.push({ kind: "assistant", text: "", state: 0 });
    this.assistant.text += text;
  }

  /** Text of a model message. */
  private text(delta: string) {
    if (this.messageStart < 0) this.messageStart = this.assistant?.text.length ?? 0;
    this.appendText(delta);
  }

  /** A model message starts: its text joins the answer after a paragraph break. */
  private messageBegin() {
    this.messageStart = this.assistant?.text.length ?? 0;
    if (this.assistant?.text) this.assistant.text += "\n\n";
  }

  /**
   * The end of a model message (`toolUse` also when its tool call starts):
   * the text of a message that calls a tool is a progress note, not the answer.
   */
  private messageEnd(stopReason: string) {
    const start = this.messageStart;
    this.messageStart = -1;
    if (stopReason !== "toolUse" || start < 0 || !this.assistant) return;
    const note = this.assistant.text.slice(start).trim();
    this.assistant.text = this.assistant.text.slice(0, start).replace(/\s+$/, "");
    if (!this.assistant.text) {
      this.rows.splice(this.rows.indexOf(this.assistant), 1);
      this.assistant = undefined;
    }
    if (note) this.lastNote = note;
  }

  /** Closes the current turn: no more text or calls join its rows. */
  endTurn() {
    this.messageStart = -1;
    // A turn that ends without an answer (the summary came with the last tool
    // call; Stop, a timeout or an error after it): its latest note is the answer.
    if (!this.assistant && this.lastNote) this.assistant = this.push({ kind: "assistant", text: this.lastNote, state: 1 });
    this.lastNote = "";
    if (this.assistant) this.assistant.state = 1;
    const row = this.activityRow;
    if (row) {
      row.seconds = this.live ? Math.max(1, Math.round((this.now() - this.startedAt) / 1000)) : 0;
      // A turn with no step keeps its row only when it was timed and the model thought.
      if (!this.calls.length && !(this.live && this.thought)) this.rows.splice(this.rows.indexOf(row), 1);
    }
    this.live = false;
    this.refreshActivity();
    this.activityRow = undefined;
    this.assistant = undefined;
    this.calls = [];
    this.thought = false;
  }

  /** A live event from the worker. */
  apply(event: WorkerEvent) {
    switch (event.kind) {
      case "assistant_start":
        this.messageBegin();
        break;
      case "text":
        this.text(event.delta);
        break;
      case "thinking":
        this.thinking(event.delta);
        break;
      case "assistant_end":
        this.messageEnd(event.stopReason);
        if (this.assistant) this.assistant.text = this.assistant.text.replace(/\s+$/, "");
        if (event.error) {
          this.endTurn();
          this.error(event.error);
        } else if (event.stopReason === "aborted") {
          this.endTurn();
          this.notice("stopped");
        }
        break;
      case "tool_start":
        this.messageEnd("toolUse");
        this.toolCall(event.id, { tool: event.name, target: event.target, reference: !!event.reference }, 0);
        break;
      case "tool_end":
        this.toolCall(event.id, NO_CALL, event.ok ? 1 : 2);
        break;
      case "retry":
        // pi retries by itself: the failed attempt's error row gives way.
        if (this.rows.at(-1)?.kind === "error") this.rows.pop();
        this.notice("retry", [event.attempt, event.max]);
        break;
      case "compaction":
        this.notice(event.done ? "compacted" : "compacting");
        break;
      case "settled":
        this.endTurn();
        break;
    }
  }

  toggle(id: string) {
    const row = this.rows.find((r) => r.id === id);
    if (row) row.expanded = !row.expanded;
  }

  /** Rebuilds the rows from pi's session JSONL (one JSON entry per line). */
  static fromJsonl(text: string): Chat {
    const chat = new Chat();
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      let entry: { type?: string; message?: Record<string, unknown> };
      try {
        entry = JSON.parse(line);
      } catch {
        continue; // a line cut short by a crash
      }
      const message = entry.message;
      if (entry.type !== "message" || !message) continue;
      const content = message.content;
      const parts = Array.isArray(content) ? (content as Record<string, unknown>[]) : [];
      if (message.role === "user") chat.user(typeof content === "string" ? content : parts.filter((p) => p.type === "text").map((p) => String(p.text)).join(""));
      else if (message.role === "assistant") {
        chat.apply({ kind: "assistant_start" });
        for (const part of parts) {
          if (part.type === "text") chat.text(String(part.text ?? ""));
          else if (part.type === "thinking") chat.thinking(String(part.thinking ?? ""));
          else if (part.type === "toolCall") {
            const args = (part.arguments ?? {}) as Record<string, unknown>;
            const target = ["path", "pattern", "query", "file", "id", "name"].map((k) => args[k]).find((v) => typeof v === "string") as string | undefined;
            chat.messageEnd("toolUse");
            chat.toolCall(String(part.id), { tool: String(part.name), target: target ?? "", reference: false }, 1);
          }
        }
        chat.messageEnd(String(message.stopReason ?? "stop"));
        if (message.stopReason === "error") {
          chat.endTurn();
          chat.error(String(message.errorMessage ?? ""));
        } else if (message.stopReason === "aborted") {
          chat.endTurn();
          chat.notice("stopped");
        }
      } else if (message.role === "toolResult" && message.isError) chat.toolCall(String(message.toolCallId), NO_CALL, 2);
    }
    chat.endTurn();
    return chat;
  }
}
