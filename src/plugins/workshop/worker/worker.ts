// The agent worker: one pi session in a Node process started by the plugin
// with the permission model on (see agent.ts for the flags). It talks only
// over the IPC channel and exits when the channel closes.
//
// pi runs fully in memory except the session JSONL: credentials in an
// InMemoryCredentialStore (no auth.json), settings in memory, no model
// catalog file, no discovered extensions, skills, prompts, themes or context
// files. Telemetry and network catalog refreshes are off (PI_TELEMETRY=0,
// PI_OFFLINE=1, set by the plugin).
import {
  InMemoryCredentialStore,
  ModelRuntime,
  SessionManager,
  SettingsManager,
  createAgentSession,
  createExtensionRuntime,
  type AgentSessionEvent,
  type ResourceLoader,
} from "../vendor/pi/pi.mjs";
import path from "node:path";
import { resolveModel } from "./models.ts";
import { pluginTools } from "./plugin-tools.ts";
import { type StartMessage, type ToPlugin, type ToolReply, type ToWorker, type WorkerEvent } from "./protocol.ts";
import { Folders, TOOL_NAMES, createTools } from "./tools.ts";

type Session = Awaited<ReturnType<typeof createAgentSession>>["session"];

let session: Session | undefined;
let secret = "";
/** Plugin tool calls waiting for the plugin's answer. */
const waiting = new Map<string, (reply: ToolReply) => void>();
let calls = 0;

function askPlugin(name: string, args: Record<string, unknown>, signal: AbortSignal | undefined): Promise<ToolReply> {
  const call = `c${++calls}`;
  return new Promise((resolve) => {
    if (signal?.aborted) return resolve({ ok: false, text: "Cancelled" });
    const abort = () => {
      waiting.delete(call);
      send({ type: "toolCancel", call });
      resolve({ ok: false, text: "Cancelled" });
    };
    signal?.addEventListener("abort", abort, { once: true });
    waiting.set(call, (reply) => {
      signal?.removeEventListener("abort", abort);
      resolve(reply);
    });
    send({ type: "tool", call, name, args });
  });
}

/** No message leaves the worker with the key in it. */
const redact = (text: string) => (secret.length >= 4 ? text.split(secret).join("••••") : text);
function send(message: ToPlugin) {
  if (!process.connected) return;
  process.send?.(JSON.parse(redact(JSON.stringify(message))));
}
const event = (e: WorkerEvent) => send({ type: "event", event: e });
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

function textOf(content: unknown): string {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((part) => part && part.type === "text" && typeof part.text === "string")
    .map((part) => part.text)
    .join("");
}

/** The reference folders (StartMessage.docsRoots), for short labels. */
let docsRoots: string[] = [];

/**
 * The file, pattern or plugin a tool call is about, for its one-line label;
 * `reference`: a file of the reference, by its short name (the window says so).
 */
function targetOf(args: unknown): { target: string; reference: boolean } {
  if (!args || typeof args !== "object") return { target: "", reference: false };
  const value = args as Record<string, unknown>;
  const p = typeof value.path === "string" ? value.path : "";
  const root = p && docsRoots.find((r) => p === r || p.startsWith(r + path.sep) || p.startsWith(r + "/"));
  if (root) return { target: `${path.basename(root)}${p.slice(root.length).split(path.sep).join("/")}`, reference: true };
  // The short name, sdk/… (see Folders).
  const short = p.replace(/^@/, "").replace(/^\.\//, "");
  if (docsRoots.some((r) => short === path.basename(r) || short.startsWith(`${path.basename(r)}/`))) return { target: short, reference: true };
  for (const key of ["path", "pattern", "file", "id", "name"]) if (typeof value[key] === "string") return { target: value[key] as string, reference: false };
  return { target: "", reference: false };
}

function forward(e: AgentSessionEvent) {
  switch (e.type) {
    case "message_start":
      if (e.message.role === "assistant") event({ kind: "assistant_start" });
      break;
    case "message_update":
      if (e.assistantMessageEvent.type === "text_delta") event({ kind: "text", delta: e.assistantMessageEvent.delta });
      else if (e.assistantMessageEvent.type === "thinking_delta") event({ kind: "thinking", delta: e.assistantMessageEvent.delta });
      break;
    case "message_end":
      if (e.message.role === "assistant") {
        const message = e.message as { content: unknown; stopReason?: string; errorMessage?: string };
        event({
          kind: "assistant_end",
          text: textOf(message.content),
          stopReason: message.stopReason ?? "stop",
          error: message.stopReason === "error" ? (message.errorMessage ?? "Unknown error") : null,
        });
        if (session?.sessionFile) send({ type: "file", sessionFile: session.sessionFile });
      }
      break;
    case "tool_execution_start":
      event({ kind: "tool_start", id: e.toolCallId, name: e.toolName, ...targetOf(e.args) });
      break;
    case "tool_execution_end":
      event({ kind: "tool_end", id: e.toolCallId, name: e.toolName, ok: !e.isError, summary: textOf(e.result?.content).slice(0, 2000) });
      break;
    case "auto_retry_start":
      event({ kind: "retry", attempt: e.attempt, max: e.maxAttempts, error: e.errorMessage });
      break;
    case "compaction_start":
      event({ kind: "compaction", done: false });
      break;
    case "compaction_end":
      event({ kind: "compaction", done: true });
      break;
    case "agent_settled":
      event({ kind: "settled", aborted: e.aborted });
      break;
  }
}

async function start(message: StartMessage) {
  secret = message.key;
  docsRoots = message.docsRoots ?? [];
  const folders = await Folders.open(message.workDir, docsRoots);
  const folder = folders.work;
  const modelRuntime = await ModelRuntime.create({
    credentials: new InMemoryCredentialStore(),
    modelsPath: null,
    refreshOnCreate: false,
  });
  if (message.baseUrl && message.api) {
    modelRuntime.registerProvider(message.provider, {
      name: message.provider,
      baseUrl: message.baseUrl,
      api: message.api,
      models: [
        {
          type: "chat",
          id: message.modelId,
          name: message.modelId,
          reasoning: !!message.reasoning,
          input: ["text"],
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
          contextWindow: 128000,
          // Thinking and the answer share max_tokens (pi takes the thinking
          // budget out of it: low 2048, medium 8192 tokens), so a reasoning
          // model gets room for a full answer with a tool call after it.
          maxTokens: message.reasoning ? 16384 : 8192,
        },
      ],
    } as Parameters<ModelRuntime["registerProvider"]>[1]);
  }
  const model = resolveModel(modelRuntime, message.provider, message.modelId);
  if (!model) throw new Error(`Unknown model: ${message.provider}/${message.modelId}`);
  // Local servers (Ollama, LM Studio) take any key.
  await modelRuntime.setRuntimeApiKey(message.provider, message.key || "none");

  const resourceLoader: ResourceLoader = {
    getExtensions: () => ({ extensions: [], errors: [], runtime: createExtensionRuntime() }),
    getSkills: () => ({ skills: [], diagnostics: [] }),
    getPrompts: () => ({ prompts: [], diagnostics: [] }),
    getThemes: () => ({ themes: [], diagnostics: [] }),
    getAgentsFiles: () => ({ agentsFiles: [] }),
    getSystemPrompt: () => message.systemPrompt,
    getSystemPromptSource: () => undefined,
    getAppendSystemPrompt: () => [],
    getAppendSystemPromptSources: () => [],
    extendResources: () => {},
    reload: async () => {},
  } as unknown as ResourceLoader;

  const sessionManager = message.sessionFile
    ? SessionManager.open(message.sessionFile, message.sessionDir, folder.root)
    : SessionManager.create(folder.root, message.sessionDir);
  ({ session } = await createAgentSession({
    cwd: folder.root,
    // Never written: credentials, settings and models are in memory. The
    // worker may not write there anyway (only the session folder).
    agentDir: message.sessionDir,
    model,
    // pi clamps the level to what the model supports ("off" for one that does not reason).
    thinkingLevel: message.thinking ?? "low",
    modelRuntime,
    resourceLoader,
    sessionManager,
    // Two automatic retries for transient failures (1.5 s, 3 s), then the error row with Retry.
    settingsManager: SettingsManager.inMemory({ retry: { enabled: true, maxRetries: 2, baseDelayMs: 1500 } }),
    noTools: "builtin",
    customTools: createTools(folders, pluginTools(message.pluginTools ?? [], askPlugin)),
    tools: [...TOOL_NAMES, ...(message.pluginTools ?? [])],
  }));
  session.subscribe(forward);
  send({ type: "ready", sessionFile: session.sessionFile ?? null });
}

async function prompt(text: string) {
  if (!session) throw new Error("The session is not ready yet");
  try {
    await session.prompt(text);
  } finally {
    if (session.sessionFile) send({ type: "file", sessionFile: session.sessionFile });
  }
}

process.on("message", (raw) => {
  const message = raw as ToWorker;
  const run = async () => {
    switch (message.type) {
      case "start":
        return start(message);
      case "prompt":
        return prompt(message.text);
      case "abort":
        return session?.abort();
      case "toolResult": {
        const done = waiting.get(message.call);
        waiting.delete(message.call);
        return done?.(message.reply);
      }
      case "shutdown":
        session?.dispose();
        process.exit(0);
    }
  };
  run().catch((error) => {
    // Details for the plugin log (stderr); the message for the user.
    const resource = (error as { resource?: string }).resource;
    process.stderr.write(redact(`${error instanceof Error ? error.stack : error}${resource ? `\nresource: ${resource}` : ""}\n`));
    send({ type: "failed", message: errorText(error) });
  });
});
// The plugin process ended or closed the channel: nothing left to answer.
process.on("disconnect", () => process.exit(0));
