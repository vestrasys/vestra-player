// The window's `rows` value: the conversation rows with each assistant
// message as a Markdown document (`body`, shown by MarkdownBlocks).
//
// - One MarkdownRef per message, made again only when its text changed: while
//   a reply streams that is the streaming message, once per refresh tick.
//   The player keeps unchanged documents parsed and updates the rest in place.
// - remoteImages: false always. Model output is untrusted: an image address
//   in a reply could carry data out (a prompt-injection channel), and a
//   tracking pixel would reveal the user. Images show as their alt text.
// - Every row has the same shape (body included, empty for rows that are not
//   Markdown), so the player can update the list in place.
// - The player's limits for one property value: ≤ 1024 rows, ≤ 4096 nodes,
//   ≤ 64 KiB per string, ≤ 1 MiB of text in all (documents included). The
//   newest rows that fit a smaller budget are sent; a notice says how many
//   older ones are not shown (they stay in the session's JSONL).
// - An activity row's steps are sent only while it is expanded (each step is
//   a few value nodes); a message cut short says so (`truncated`, ai.slint).
import type { MarkdownRef } from "@vestra-player/sdk";
import type { Row, ToolCall } from "./chat.ts";

/** Rows sent at most (each row is about 10 value nodes). */
export const MAX_ROWS = 200;
/** Text bytes sent at most, documents included (the player allows 1 MiB). */
export const MAX_TEXT_BYTES = 900 * 1024;
/** One message's document at most (the player allows 256 KB). */
export const MAX_DOC_BYTES = 200 * 1024;
/** A plain string field at most (the player allows 64 KiB). */
export const MAX_FIELD_BYTES = 16 * 1024;
/** Value nodes sent at most (the player allows 4096). */
export const MAX_NODES = 3500;
/** Value nodes of a row without its calls, and of one call. */
const ROW_NODES = 24;
const CALL_NODES = 5;

const bytes = (text: string) => Buffer.byteLength(text);

/** `text` cut to at most `limit` UTF-8 bytes, on a character boundary. */
export function cut(text: string, limit: number): string {
  if (bytes(text) <= limit) return text;
  let out = "";
  let size = 0;
  for (const char of text) {
    size += bytes(char);
    if (size > limit) break;
    out += char;
  }
  return out;
}

/** A type, not an interface: property values need an index-compatible shape. */
export type PropRow = {
  id: string;
  kind: string;
  text: string;
  detail: string;
  state: number;
  expanded: boolean;
  settings: boolean;
  body: MarkdownRef;
  code: string;
  args: string[];
  n: number;
  seconds: number;
  calls: ToolCall[];
  call: ToolCall;
  failed: number;
  /** The message is longer than shown. */
  truncated: boolean;
};

const textLimit = (row: Row) => (row.kind === "assistant" ? MAX_DOC_BYTES : MAX_FIELD_BYTES);
/** The steps sent for a row: only an expanded activity row lists them. */
const callsOf = (row: Row) => (row.kind === "activity" && row.expanded ? row.calls : []);

export class MessageDocs {
  private readonly docs = new Map<string, { text: string; ref: MarkdownRef }>();
  private empty: MarkdownRef | undefined;
  private running: Promise<void> | undefined;
  private again: Row[] | undefined;

  private readonly markdown: (text: string) => Promise<MarkdownRef>;
  constructor(markdown: (text: string) => Promise<MarkdownRef>) {
    this.markdown = markdown;
  }

  /** The newest rows within the budget; `hidden`: how many older ones are left out. */
  static visible(rows: Row[]): { rows: Row[]; hidden: number } {
    let total = 0;
    let nodes = 0;
    let first = rows.length;
    while (first > 0 && rows.length - first < MAX_ROWS) {
      const row = rows[first - 1];
      const calls = callsOf(row);
      const size =
        bytes(cut(row.text, textLimit(row))) +
        bytes(cut(row.detail, MAX_FIELD_BYTES)) +
        row.args.reduce((sum, arg) => sum + bytes(arg), 0) +
        [...calls, row.call].reduce((sum, call) => sum + bytes(call.tool) + bytes(call.target), 0);
      const count = ROW_NODES + calls.length * CALL_NODES;
      if ((total + size > MAX_TEXT_BYTES || nodes + count > MAX_NODES) && first < rows.length) break;
      total += size;
      nodes += count;
      first--;
    }
    return { rows: rows.slice(first), hidden: first };
  }

  /**
   * Makes the documents of the visible assistant rows current. Calls while
   * one runs are folded into one more pass with the latest rows.
   */
  sync(rows: Row[]): Promise<void> {
    if (this.running) {
      this.again = rows;
      return this.running;
    }
    this.running = (async () => {
      let next: Row[] | undefined = rows;
      while (next) {
        this.again = undefined;
        await this.pass(next);
        next = this.again;
      }
    })().finally(() => (this.running = undefined));
    return this.running;
  }

  private async pass(rows: Row[]) {
    this.empty ??= await this.markdown("");
    const shown = MessageDocs.visible(rows).rows.filter((row) => row.kind === "assistant");
    const keep = new Set(shown.map((row) => row.id));
    for (const id of this.docs.keys()) if (!keep.has(id)) this.docs.delete(id);
    for (const row of shown) {
      const text = cut(row.text, MAX_DOC_BYTES);
      if (this.docs.get(row.id)?.text !== text) this.docs.set(row.id, { text, ref: await this.markdown(text) });
    }
  }

  /** The value for the window's `rows` (synchronous: documents not made yet show as plain text). */
  props(rows: Row[]): PropRow[] {
    const { rows: shown, hidden } = MessageDocs.visible(rows);
    const empty = this.empty;
    const out: PropRow[] = [];
    if (!empty) return out; // before the first sync(); it runs before the window opens
    const none = { tool: "", target: "", reference: false, state: 0 };
    if (hidden)
      out.push({
        id: "hidden",
        kind: "notice",
        text: "",
        detail: "",
        state: 1,
        expanded: false,
        settings: false,
        body: empty,
        code: "hidden",
        args: [],
        n: hidden,
        seconds: 0,
        calls: [],
        call: none,
        failed: 0,
        truncated: false,
      });
    for (const row of shown) {
      const doc = row.kind === "assistant" ? this.docs.get(row.id) : undefined;
      out.push({
        id: row.id,
        kind: row.kind,
        text: doc ? "" : cut(row.text, MAX_FIELD_BYTES),
        detail: cut(row.detail, MAX_FIELD_BYTES),
        state: row.state,
        expanded: row.expanded,
        settings: row.settings,
        body: doc?.ref ?? empty,
        code: row.code,
        args: row.args,
        n: row.n,
        seconds: row.seconds,
        calls: callsOf(row),
        call: row.call,
        failed: row.failed,
        truncated: bytes(row.text) > (doc ? MAX_DOC_BYTES : MAX_FIELD_BYTES),
      });
    }
    return out;
  }
}
