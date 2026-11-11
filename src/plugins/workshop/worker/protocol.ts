// Messages between the plugin process and the agent worker (Node IPC, JSON).
// Types only: the plugin's build copies worker/ as it is and transpiles the
// plugin side, so the two sides share no runtime module.

export interface StartMessage {
  type: "start";
  /** sessions/<sid>/: pi's JSONL lives here. */
  sessionDir: string;
  /** sessions/<sid>/work/: the only folder the tools see. */
  workDir: string;
  /** The session's JSONL to continue, or null for a new conversation. */
  sessionFile: string | null;
  provider: string;
  modelId: string;
  /**
   * A custom endpoint, registered as `provider`: its Base URL and wire format.
   * Both null for pi's built-in providers.
   */
  baseUrl: string | null;
  api: "openai-completions" | "anthropic-messages" | null;
  /** The API key, from the keychain; kept in the worker's memory only. */
  key: string;
  systemPrompt: string;
  /** pi's thinking level for the session; a model that does not reason runs without (pi clamps it). */
  thinking: "off" | "low" | "medium";
  /** A custom endpoint's model reasons (pi's catalog says so for its own providers: null). */
  reasoning: boolean | null;
  /**
   * Read-only folders the file tools may also read (the SDK package and the
   * generated reference): real paths, outside the session folder.
   */
  docsRoots: string[];
  /** Plugin tools the plugin process runs for the agent (PLUGIN_TOOLS names; ASK_TOOLS names for "ask"). */
  pluginTools: string[];
  /**
   * "maker" (default): the plugin maker's chat, with the file and
   * documentation tools and the session JSONL. "ask": one Ask Vestra request,
   * kept in memory only (no session file), without file or documentation
   * tools: only the ASK_TOOLS named in `pluginTools`, whose result ends the
   * turn (no model call after it).
   */
  profile?: "maker" | "ask";
}

/** What a plugin tool call returned (the plugin process ran it). */
export interface ToolReply {
  ok: boolean;
  text: string;
}

export type ToWorker =
  | StartMessage
  | { type: "prompt"; text: string }
  | { type: "abort" }
  | { type: "toolResult"; call: string; reply: ToolReply }
  | { type: "shutdown" };

export type WorkerEvent =
  | { kind: "assistant_start" }
  | { kind: "text"; delta: string }
  | { kind: "thinking"; delta: string }
  | { kind: "assistant_end"; text: string; stopReason: string; error: string | null }
  | { kind: "tool_start"; id: string; name: string; target: string; /** `target` is a reference file (sdk/…). */ reference?: boolean }
  | { kind: "tool_end"; id: string; name: string; ok: boolean; summary: string }
  | { kind: "retry"; attempt: number; max: number; error: string }
  | { kind: "compaction"; done: boolean }
  | { kind: "settled"; aborted: boolean };

export type ToPlugin =
  | { type: "ready"; sessionFile: string | null }
  | { type: "event"; event: WorkerEvent }
  /** The session's JSONL once pi created it (after the first prompt). */
  | { type: "file"; sessionFile: string }
  | { type: "failed"; message: string }
  /** Run plugin tool `name` (the plugin answers with toolResult). */
  | { type: "tool"; call: string; name: string; args: Record<string, unknown> }
  /** The agent stopped waiting for `call` (aborted): cancel it. */
  | { type: "toolCancel"; call: string };
