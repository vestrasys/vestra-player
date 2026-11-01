// The rows of one conversation as the window shows them, built from the
// worker's live events or from pi's JSONL. One user turn is: the user's
// message, ONE collapsed "thought" row (the model's thinking and the notes it
// wrote between tool calls: "Thought for 12 s", expandable), ONE rolling tool
// line (the latest call replaces the previous one; expandable to the whole
// list), then the assistant's answer, as ChatGPT, Claude and Cursor show it.
// The answer is the text of model messages that did not end in a tool call;
// the text of one that did is a progress note and moves to the thought row
// when its tool call comes (while it streams it shows as the answer).
//
// Rows carry data, not sentences: notices, errors and the tool line say what
// happened with a `code` and its `args`; ai.slint has the words (@tr).
import type { WorkerEvent } from "./worker/protocol.ts";

/** One tool call: its tool, what it is about, 0 running / 1 done / 2 failed. */
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
  /** "user" | "thought" | "tools" | "assistant" | "notice" | "error" */
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
   * notice / error: what to say (see ai.slint). tools: "running" (`call` in
   * progress), "after" (`call` ended, the model `phase`), "phase" (no call
   * yet), "done" (the turn is over: `n` calls, `failed` failed).
   * thought: "thinking" (now), "thought" (`n` seconds), "thought-untimed"
   * (from the record, which has no durations), "notes" (`n` notes, no thinking).
   */
  code: string;
  args: string[];
  /** notice: the count of a plural notice; tools: the number of calls. */
  n: number;
  /** tools: every call of the turn, and the one the line shows. */
  calls: ToolCall[];
  call: ToolCall;
  /** tools: "thinking" | "replying" while the turn runs. */
  phase: string;
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
  private toolsRow: Row | undefined;
  private assistant: Row | undefined;
  /** A live turn is in progress (the status line says what the agent is doing). */
  private live = false;
  /** The latest model output of the live turn is text (not a tool call). */
  private replying = false;
  /** The turn's thought row: thinking and progress notes. */
  private thought: Row | undefined;
  /** What the thought row's text ends with, to start a paragraph when it changes. */
  private segment: "" | "thinking" | "note" = "";
  /** When the current stretch of thinking began (0: not thinking), and the turn's total. */
  private thinkingSince = 0;
  private thinkingMs = 0;
  private thinkingSeen = false;
  /** Thinking came from the record (no durations). */
  private untimed = false;
  private notes = 0;
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
    return { id: nextId(), text: "", detail: "", state: 1, expanded: false, settings: false, code: "", args: [], n: 0, calls: [], call: NO_CALL, phase: "", failed: 0, ...fields };
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
   * A live turn starts (a message was sent): the status line appears at once
   * as "Thinking…", before the model's first event, and follows every event.
   */
  begin() {
    this.live = true;
    this.replying = false;
    this.statusRow();
    this.refreshTools();
  }

  private statusRow(): Row {
    if (!this.toolsRow) {
      const row = Chat.row({ kind: "tools", state: 0 });
      // Before the turn's text: the line reads as "what the agent did".
      const at = this.assistant ? this.rows.indexOf(this.assistant) : -1;
      if (at >= 0) this.rows.splice(at, 0, row);
      else this.rows.push(row);
      this.toolsRow = row;
    }
    return this.toolsRow;
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

  private refreshTools() {
    const row = this.toolsRow;
    if (!row) return;
    const running = this.calls.findLast((call) => call.state === 0);
    const last = this.calls.at(-1);
    const failed = this.calls.filter((call) => call.state === 2).length;
    const plain = ({ tool, target, reference, state }: ToolCall): ToolCall => ({ tool, target, reference, state });
    if (running || this.live) {
      // While the turn runs: the call in progress, else the latest call and
      // what the model does now; each new call replaces the previous one.
      row.state = 0;
      row.phase = this.replying ? "replying" : "thinking";
      row.code = running ? "running" : last ? "after" : "phase";
      row.call = running ? plain(running) : last ? plain(last) : NO_CALL;
    } else {
      row.state = failed ? 2 : 1;
      row.code = "done";
      row.phase = "";
      row.call = NO_CALL;
    }
    row.n = this.calls.length;
    row.failed = failed;
    row.calls = this.calls.map(plain);
  }

  private toolCall(id: string, call: Omit<ToolCall, "state">, state: number) {
    this.statusRow();
    if (state === 0) this.replying = false;
    const known = this.calls.find((c) => c.id === id);
    if (known) known.state = state;
    else this.calls.push({ id, ...call, state });
    this.refreshTools();
  }

  private thoughtRow(): Row {
    if (!this.thought) {
      const row = Chat.row({ kind: "thought", state: 0 });
      // First in the turn: before the tool line and the answer.
      const before = this.toolsRow ?? this.assistant;
      const at = before ? this.rows.indexOf(before) : -1;
      if (at >= 0) this.rows.splice(at, 0, row);
      else this.rows.push(row);
      this.thought = row;
    }
    return this.thought;
  }

  private refreshThought() {
    const row = this.thought;
    if (!row) return;
    const thinking = this.thinkingSince > 0;
    row.state = thinking ? 0 : 1;
    row.code = thinking ? "thinking" : !this.thinkingSeen ? "notes" : this.untimed ? "thought-untimed" : "thought";
    row.n = this.thinkingSeen ? Math.max(1, Math.round(this.thinkingMs / 1000)) : this.notes;
  }

  private addThought(kind: "thinking" | "note", text: string) {
    const row = this.thoughtRow();
    if (this.segment !== kind || kind === "note") {
      if (row.text) row.text += "\n\n";
      this.segment = kind;
    }
    row.text += text;
    this.refreshThought();
  }

  /** Thinking text of the model; `timed`: live (its duration counts). */
  private thinking(delta: string, timed: boolean) {
    if (!delta) return;
    this.thinkingSeen = true;
    if (!timed) this.untimed = true;
    else if (!this.thinkingSince) this.thinkingSince = this.now();
    this.addThought("thinking", delta);
  }

  /** A stretch of thinking ended (text, a tool call or the end of the message came). */
  private stopThinking() {
    if (!this.thinkingSince) return;
    this.thinkingMs += this.now() - this.thinkingSince;
    this.thinkingSince = 0;
    this.refreshThought();
  }

  private appendText(text: string) {
    if (!text) return;
    this.stopThinking();
    if (this.live && !this.replying) {
      this.replying = true;
      this.refreshTools();
    }
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
    // Its thinking is a paragraph of its own in the thought row.
    this.segment = "";
  }

  /**
   * The end of a model message (`toolUse` also when its tool call starts):
   * the text of a message that calls a tool is a progress note, not the answer.
   */
  private messageEnd(stopReason: string) {
    this.stopThinking();
    const start = this.messageStart;
    this.messageStart = -1;
    if (stopReason !== "toolUse" || start < 0 || !this.assistant) return;
    const note = this.assistant.text.slice(start).trim();
    this.assistant.text = this.assistant.text.slice(0, start).replace(/\s+$/, "");
    if (!this.assistant.text) {
      this.rows.splice(this.rows.indexOf(this.assistant), 1);
      this.assistant = undefined;
    }
    if (note) {
      this.notes++;
      this.lastNote = note;
      this.addThought("note", note);
    }
  }

  /**
   * A turn that ends without an answer (the summary came with the last tool
   * call; Stop, a timeout or an error after it): its latest note is the answer.
   */
  private restoreAnswer() {
    const row = this.thought;
    const note = this.lastNote;
    if (this.assistant || !note || !row) return;
    if (row.text.endsWith(note)) row.text = row.text.slice(0, row.text.length - note.length).replace(/\s+$/, "");
    this.notes--;
    this.refreshThought();
    if (!row.text) this.rows.splice(this.rows.indexOf(row), 1);
    this.assistant = this.push({ kind: "assistant", text: note, state: 1 });
  }

  /** Closes the current turn: no more text or calls join its rows. */
  endTurn() {
    this.stopThinking();
    this.messageStart = -1;
    this.restoreAnswer();
    this.lastNote = "";
    if (this.assistant) this.assistant.state = 1;
    this.thought = undefined;
    this.segment = "";
    this.thinkingMs = 0;
    this.thinkingSeen = false;
    this.untimed = false;
    this.notes = 0;
    this.live = false;
    // A turn without tool calls keeps no status line.
    if (this.toolsRow && !this.calls.length) this.rows.splice(this.rows.indexOf(this.toolsRow), 1);
    else this.refreshTools();
    this.toolsRow = undefined;
    this.assistant = undefined;
    this.calls = [];
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
        this.thinking(event.delta, true);
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
        this.stopThinking();
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
          else if (part.type === "thinking") chat.thinking(String(part.thinking ?? ""), false);
          else if (part.type === "toolCall") {
            const args = (part.arguments ?? {}) as Record<string, unknown>;
            const target = ["path", "pattern", "file", "id", "name"].map((k) => args[k]).find((v) => typeof v === "string") as string | undefined;
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
