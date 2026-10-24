// The rows of one conversation as the window shows them, built from the
// worker's live events or from pi's JSONL. One user turn is: the user's
// message, ONE rolling tool line (the latest call replaces the previous one;
// expandable to the whole list), then the assistant's text.
import type { WorkerEvent } from "./worker/protocol.ts";

export interface Row {
  id: string;
  /** "user" | "tools" | "assistant" | "notice" | "error" */
  kind: string;
  text: string;
  /** tools: one line per call; error: the provider's message. */
  detail: string;
  /** 0 running, 1 done, 2 failed */
  state: number;
  expanded: boolean;
  /** error rows: offer 「打开设置」. */
  settings: boolean;
}

interface Call {
  id: string;
  label: string;
  state: number;
}

const VERBS: Record<string, string> = {
  read: "读取",
  write: "写入",
  edit: "修改",
  ls: "列出",
  find: "查找",
  grep: "搜索",
  scaffold_plugin: "创建插件",
  typecheck: "类型检查",
  test: "运行测试",
  check: "检查",
  pack: "打包",
  trial_load: "试载",
  install_plugin: "安装",
};

export function toolLabel(name: string, target: string): string {
  const verb = VERBS[name] ?? name;
  return target ? `${verb} ${target}` : verb;
}

/** A provider or network failure in Chinese, and whether the key is the likely cause. */
export function classify(message: string, model = ""): { text: string; settings: boolean } {
  if (/\b(401|403)\b|invalid.{0,20}(api.?key|x-api-key|token)|incorrect api key|authentication|authenticate|unauthori[sz]ed/i.test(message))
    return { text: "API key 无效或没有权限，请在设置中检查。", settings: true };
  if (
    /model.{0,60}(not found|no longer available|not available|does not exist|not exist|is not supported|unsupported|deprecated|retired)|(unknown|invalid|unsupported) model|model_not_found|未知模型/i.test(message)
  )
    return { text: model ? `模型「${model}」不可用，请在设置中换一个模型。` : "模型不可用，请在设置中换一个模型。", settings: true };
  if (/\b429\b|quota|insufficient|credit|billing|rate.?limit/i.test(message)) return { text: "额度不足或请求过于频繁，请稍后重试。", settings: false };
  if (/ECONNREFUSED|ENOTFOUND|EAI_AGAIN|ECONNRESET|ETIMEDOUT|fetch failed|network|socket|getaddrinfo|Connection error/i.test(message))
    return { text: "网络错误：无法连接服务商。", settings: false };
  return { text: "出错了。", settings: false };
}

let serial = 0;
const nextId = () => `r${++serial}`;

export class Chat {
  rows: Row[] = [];
  private calls: Call[] = [];
  private toolsRow: Row | undefined;
  private assistant: Row | undefined;
  /** A live turn is in progress (the status line says what the agent is doing). */
  private live = false;
  /** The latest model output of the live turn is text (not a tool call). */
  private replying = false;
  /** The text of the last user message, for 「重试」. */
  lastPrompt = "";
  /** The model of the current run, named in a "model not available" error. */
  model = "";

  private push(row: Omit<Row, "id" | "detail" | "state" | "expanded" | "settings"> & Partial<Row>): Row {
    const full: Row = { id: nextId(), detail: "", state: 1, expanded: false, settings: false, ...row };
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
   * as 「思考中…」, before the model's first event, and follows every event.
   */
  begin() {
    this.live = true;
    this.replying = false;
    this.statusRow();
    this.refreshTools();
  }

  private statusRow(): Row {
    if (!this.toolsRow) {
      const row: Row = { id: nextId(), kind: "tools", text: "", detail: "", state: 0, expanded: false, settings: false };
      // Before the turn's text: the line reads as "what the agent did".
      const at = this.assistant ? this.rows.indexOf(this.assistant) : -1;
      if (at >= 0) this.rows.splice(at, 0, row);
      else this.rows.push(row);
      this.toolsRow = row;
    }
    return this.toolsRow;
  }

  notice(text: string) {
    this.push({ kind: "notice", text });
  }

  error(message: string, override?: string) {
    const { text, settings } = classify(message, this.model);
    this.push({ kind: "error", text: override ?? text, detail: message.split("\n")[0].slice(0, 400), state: 2, settings });
  }

  private refreshTools() {
    const row = this.toolsRow;
    if (!row) return;
    const running = this.calls.findLast((call) => call.state === 0);
    const last = this.calls.at(-1);
    const failed = this.calls.filter((call) => call.state === 2).length;
    if (running || this.live) {
      // While the turn runs: the call in progress, else the latest call and
      // what the model does now; each new call replaces the previous one.
      row.state = 0;
      const now = this.replying ? "回复中…" : "思考中…";
      row.text = running ? `${running.label}…` : last ? `${last.label}${last.state === 2 ? " ✗" : " ✓"} · ${now}` : now;
    } else {
      row.state = failed ? 2 : 1;
      row.text = `${this.calls.length} 个工具调用${failed ? `，${failed} 个失败` : ""}`;
    }
    row.detail = this.calls.map((call) => `${call.state === 0 ? "…" : call.state === 1 ? "✓" : "✗"} ${call.label}`).join("\n");
  }

  private toolCall(id: string, label: string, state: number) {
    this.statusRow();
    if (state === 0) this.replying = false;
    const known = this.calls.find((call) => call.id === id);
    if (known) known.state = state;
    else this.calls.push({ id, label, state });
    this.refreshTools();
  }

  private appendText(text: string) {
    if (!text) return;
    if (this.live && !this.replying) {
      this.replying = true;
      this.refreshTools();
    }
    if (!this.assistant) this.assistant = this.push({ kind: "assistant", text: "", state: 0 });
    this.assistant.text += text;
  }

  /** Closes the current turn: no more text or calls join its rows. */
  endTurn() {
    if (this.assistant) this.assistant.state = 1;
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
        // Text of a new model message after tool calls starts a new paragraph.
        if (this.assistant?.text) this.assistant.text += "\n\n";
        break;
      case "text":
        this.appendText(event.delta);
        break;
      case "assistant_end":
        if (this.assistant) this.assistant.text = this.assistant.text.replace(/\s+$/, "");
        if (event.error) {
          this.endTurn();
          this.error(event.error);
        } else if (event.stopReason === "aborted") {
          this.endTurn();
          this.notice("已停止");
        }
        break;
      case "tool_start":
        this.toolCall(event.id, toolLabel(event.name, event.target), 0);
        break;
      case "tool_end":
        this.toolCall(event.id, "", event.ok ? 1 : 2);
        break;
      case "retry":
        // pi retries by itself: the failed attempt's error row gives way.
        if (this.rows.at(-1)?.kind === "error") this.rows.pop();
        this.notice(`连接失败，正在重试（${event.attempt}/${event.max}）…`);
        break;
      case "compaction":
        this.notice(event.done ? "上下文已压缩" : "正在压缩上下文…");
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
          if (part.type === "text") chat.appendText(String(part.text ?? ""));
          else if (part.type === "toolCall") {
            const args = (part.arguments ?? {}) as Record<string, unknown>;
            const target = ["path", "pattern", "file", "id", "name"].map((k) => args[k]).find((v) => typeof v === "string") as string | undefined;
            chat.toolCall(String(part.id), toolLabel(String(part.name), target ?? ""), 1);
          }
        }
        if (message.stopReason === "error") {
          chat.endTurn();
          chat.error(String(message.errorMessage ?? "未知错误"));
        } else if (message.stopReason === "aborted") {
          chat.endTurn();
          chat.notice("已停止");
        }
      } else if (message.role === "toolResult" && message.isError) chat.toolCall(String(message.toolCallId), "", 2);
    }
    chat.endTurn();
    return chat;
  }
}
