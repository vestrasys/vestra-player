import type { Plugin } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { AgentWorker } from "./agent.ts";
import { Delivering } from "./delivery.ts";
import { systemPrompt } from "./prompt.ts";
import { OwnerError, Toolchain, checkOwner, type Project, type SessionPaths } from "./toolchain.ts";
import { BUILD_TOOLS, DELIVERY_TOOLS, runTool } from "./toolhost.ts";
import { Chat } from "./chat.ts";
import { MessageDocs, cut } from "./messages.ts";
import {
  BaseUrlError,
  DEFAULT_SETTINGS,
  LEGACY_COMPAT,
  NoModelsError,
  checkBaseUrl,
  customList,
  goneName,
  listModels,
  loadProviders,
  masked,
  migrate,
  pickListed,
  reasoningKey,
  reasoningOf,
  reasons,
  say as text,
  secretName,
  THINKING,
  thinkingOf,
  validSettings,
  type AiSettings,
  type Provider,
  type Say,
} from "./providers.ts";
import { Sessions, group, titleFrom, type Meta } from "./sessions.ts";

const PI_URL = "https://github.com/earendil-works/pi";
/** No event from a running agent for this long: stop and say so. */
export const QUIET_TIMEOUT_MS = 120_000;
/** Streaming text reaches the window at most this often. */
const REFRESH_MS = 100;

interface Run {
  worker: AgentWorker;
  meta: Meta;
  chat: Chat;
  running: boolean;
  watchdog?: ReturnType<typeof setTimeout>;
  /** Plugin tool calls in progress (a test run, a dialog): no quiet timeout meanwhile. */
  tools: number;
  /** The settings it started with (see `generation` in activate). */
  generation: number;
}

/** Vestra Player AI: chat with a model of the user's choice (BYOK); the agent works in the session folder. */
export default async function activate(plugin: Plugin) {
  const providers = loadProviders(import.meta.dirname);
  const sessions = new Sessions(plugin.dataDirectory);
  // check lets the player compile the package (Slint included) before any install.
  const toolchain = new Toolchain(import.meta.dirname, plugin.dataDirectory, (file) => plugin.packages.validateArchive(file));
  const pathsOf = (id: string): SessionPaths => ({ dir: sessions.directory(id), work: sessions.work(id) });
  const settings = plugin.storage.json<AiSettings>("settings", { defaults: DEFAULT_SETTINGS, validate: validSettings });
  // Settings migrate now (a file); keys only later (see moveLegacyKeys).
  const migrated = migrate(settings.value);
  if (migrated) settings.save(migrated);
  /** The id prefix of new plugins: the user's GitHub name, "" for local. (see checkOwner). */
  const owner = () => {
    try {
      return checkOwner(settings.value.owner ?? "");
    } catch {
      return "";
    }
  };
  const delivering = new Delivering(plugin, toolchain, owner);
  /** Installs from a session; a done install is recorded in its meta.json (installed), for the delete dialog. */
  const installFrom = async (session: SessionPaths, id: string, ask: boolean, signal?: AbortSignal) => {
    const result = await delivering.install(session, id, ask, signal);
    const meta = metas.find((m) => sessions.directory(m.id) === session.dir);
    if (result.ok && meta && !meta.installed?.includes(id)) {
      meta.installed = [...(meta.installed ?? []), id];
      await sessions.save(meta).catch((error) => console.error(`meta.json: ${error}`));
    }
    return result;
  };
  const delivery = {
    install: (session: SessionPaths, id: string, signal: AbortSignal) => installFrom(session, id, true, signal),
  };
  /**
   * provider → key, read from the keychain once, on demand. Never during
   * activation: a read can wait on the system's authorization dialog
   * (macOS asks again after the player was re-signed), and activation must
   * not wait on the user.
   */
  const keys = new Map<string, string>();
  const reading = new Map<string, Promise<string>>();
  /** The keychain read failed or was denied (cleared by the next attempt or a save): the banner's text. */
  let keyError: Say | null = null;
  const runs = new Map<string, Run>(); // session id → its worker
  const chats = new Map<string, Chat>(); // session id → rows (loaded once)

  let metas: Meta[] = await sessions.list();
  let active: Meta | null = metas.find((m) => m.id === settings.value.lastSession) ?? null;
  let draft = "";
  let pageMessage: Say = text();
  let pageWarn = false;
  let keyText = "";
  /** Models a custom endpoint listed (this run only; the chosen one is saved). */
  const listed = new Map<string, string[]>();
  let listing = false;

  /**
   * The saved provider. One that is no longer offered stays selected (its
   * model, Base URL and key are left alone) as a stand-in without models:
   * nothing is sent until the user picks another one.
   */
  /** The models offered for `p`: those it listed first, then the rest of pi's catalog. */
  const modelsOf = (p: Provider) => {
    const fetched = listed.get(p.id) ?? [];
    return [...fetched, ...p.models.filter((m) => !fetched.includes(m))];
  };
  const provider = (): Provider =>
    providers.find((p) => p.id === settings.value.provider) ?? { id: settings.value.provider, label: settings.value.provider, models: [], defaultModel: "", api: null };
  const gone = () => !providers.some((p) => p.id === settings.value.provider);
  const model = () => settings.value.models[provider().id] || provider().defaultModel;
  /**
   * A custom endpoint's model reasons: the user's choice in the settings, or
   * guessed from its id. null for pi's providers (its catalog knows).
   */
  const reasoning = (): boolean | null => {
    const p = provider();
    if (!p.api) return null;
    return reasoningOf(settings.value, reasoningKey(p.id, model())) ?? reasons(model());
  };
  const thinking = () => thinkingOf(settings.value);
  /**
   * Bumped by every change of service, key, model or thinking: a worker
   * started before runs the next message with a new worker (the session
   * file continues), also one that was generating when the change came.
   */
  let generation = 0;
  /**
   * The first slice kept the custom endpoint's key under another name: move
   * it once. Idempotent and retried until it succeeds (settings.migrateKeys).
   */
  let moving: Promise<void> | undefined;
  const moveLegacyKeys = () =>
    (moving ??= (async () => {
      if (settings.value.migrateKeys === false) return;
      for (const old of LEGACY_COMPAT) {
        const key = await plugin.secrets.get(secretName(old));
        if (!key) continue;
        if (!(await plugin.secrets.get(secretName("custom-openai")))) await plugin.secrets.set(secretName("custom-openai"), key);
        await plugin.secrets.delete(secretName(old));
        markSaved("custom-openai", true);
        keys.delete("custom-openai");
      }
      settings.save({ ...settings.value, migrateKeys: false });
    })().finally(() => (moving = undefined)));
  const markSaved = (id: string, saved: boolean) => {
    const savedKeys = { ...(settings.value.savedKeys ?? {}) };
    if (saved) savedKeys[id] = true;
    else delete savedKeys[id];
    settings.save({ ...settings.value, savedKeys });
  };
  /** The key of `id`, read once; concurrent callers share the read. */
  const keyFor = (id: string): Promise<string> => {
    if (keys.has(id)) return Promise.resolve(keys.get(id)!);
    let pending = reading.get(id);
    if (!pending) {
      pending = (async () => {
        try {
          // Only the custom OpenAI entry inherits the first slice's key.
          if (id === "custom-openai") await moveLegacyKeys();
          const key = (await plugin.secrets.get(secretName(id))) ?? "";
          if (key) {
            if (!settings.value.savedKeys?.[id]) markSaved(id, true);
          } else if (settings.value.savedKeys?.[id]) {
            // Saved before, gone now (removed outside the player).
            throw Object.assign(new Error("The key is not in the keychain"), { missing: true });
          }
          keys.set(id, key);
          keyError = null;
          return key;
        } catch (error) {
          keyError = (error as { missing?: boolean }).missing ? text("key-missing") : text("key-unreadable", [(error as Error).message]);
          throw error;
        } finally {
          reading.delete(id);
          plugin.update();
        }
      })();
      reading.set(id, pending);
      plugin.update();
    }
    return pending;
  };
  /** Starts reading the current provider's key without waiting (window shown, provider chosen). */
  const loadKey = () => {
    if (gone()) return; // its key stays where it is, unread
    keyFor(provider().id).catch(() => {}); // shown through keyError
  };
  /**
   * Ready to send: a model, the key read (a custom endpoint may be a local
   * server without one, but only when the user never saved a key for it).
   */
  const configured = () => {
    const p = provider();
    if (gone() || !model() || !keys.has(p.id)) return false;
    if (p.api) return !!settings.value.baseUrls[p.id] && (!!keys.get(p.id) || !settings.value.savedKeys?.[p.id]);
    return !!keys.get(p.id);
  };
  const running = () => [...runs.values()].find((r) => r.running);
  /** The line above the conversation: keychain state or missing setup (ai.slint has the words). */
  const banner = (): Say => {
    const p = provider();
    if (gone()) return text("gone", [goneName(p.id)]);
    if (reading.has(p.id)) return text("reading");
    if (keyError) return keyError;
    if (!configured()) return !keys.has(p.id) ? text("not-read") : text("not-configured");
    return text();
  };

  /** Throttled window refresh for events that arrive outside SDK handlers. */
  let refresh: ReturnType<typeof setTimeout> | undefined;
  const later = () => {
    refresh ??= setTimeout(() => {
      refresh = undefined;
      // The streaming message gets its new document once per tick.
      docs
        .sync(shown.rows)
        .catch((error) => console.error(`markdown: ${error}`))
        .finally(() => plugin.update());
    }, REFRESH_MS);
  };

  const chatOf = async (meta: Meta): Promise<Chat> => {
    let chat = chats.get(meta.id);
    if (!chat) {
      chat = Chat.fromJsonl(await sessions.transcript(meta));
      if (meta.running) {
        // The plugin stopped during a run: the unfinished reply is lost.
        chat.notice("interrupted");
        meta.running = false;
        await sessions.save(meta);
      }
      chats.set(meta.id, chat);
    }
    return chat;
  };
  let shown: Chat = active ? await chatOf(active) : new Chat();
  const docs = new MessageDocs((text) => plugin.ui.markdown(text, { remoteImages: false }));
  await docs.sync(shown.rows);

  const sessionRows = () => {
    // A heading's title is its group, "today" or "earlier" (ai.slint names it).
    const rows: { id: string; title: string; header: boolean; active: boolean; running: boolean }[] = [];
    let heading = "";
    for (const meta of metas) {
      const name = group(meta.updated);
      if (name !== heading) rows.push({ id: `h-${name}`, title: (heading = name), header: true, active: false, running: false });
      rows.push({ id: meta.id, title: meta.title, header: false, active: meta.id === active?.id, running: !!runs.get(meta.id)?.running });
    }
    return rows;
  };

  // ---- the result card: the shown session's plugin projects
  let projects: Project[] = [];
  /** The project the card shows (the newest unless the user picked another). */
  let chosen: string | null = null;
  let exporting = false;
  const project = () => projects.find((p) => p.id === chosen) ?? null;
  const refreshProjects = () => {
    const meta = active;
    (async () => {
      const list = meta ? await toolchain.projects(pathsOf(meta.id)) : [];
      if (active !== meta) return;
      const newest = list[0]?.id;
      // A new or just changed project comes forward.
      if (!chosen || !list.some((p) => p.id === chosen) || newest !== projects[0]?.id || list[0]?.modified !== projects[0]?.modified) chosen = newest ?? null;
      projects = list;
      plugin.update();
    })().catch((error) => console.error(`projects: ${error}`));
  };
  const cardSay = (code: string, args: string[] = []) => delivering.say(code, args);
  /** A card action: failures show on the card's line. */
  const action = (work: (meta: Meta, p: Project) => Promise<unknown>) => async () => {
    const p = project();
    if (!active || !p) return;
    try {
      await work(active, p);
    } catch (error) {
      cardSay("error", [(error as Error).message]);
    } finally {
      refreshProjects();
    }
  };

  const aiWindow = plugin.windows.create("ai.slint");
  aiWindow.bind(() => {
    const busy = !!(active && runs.get(active.id)?.running);
    const last = shown.rows.at(-1);
    const p = project();
    return {
      // The card (Install, Export) is for a finished turn: hidden while the
      // agent works, back with the project's state when the turn has ended.
      hasProject: !!p && !busy,
      projects: projects.map((x) => `${x.name} · ${x.id}`),
      projectIndex: Math.max(0, projects.findIndex((x) => x.id === chosen)),
      projectTitle: p ? `${p.name} · ${p.id} ${p.version}` : "",
      cardStatus: delivering.status,
      cardKeys: delivering.shadowedKeys,
      cardBusy: delivering.busy || exporting,
      sessions: sessionRows(),
      rows: docs.props(shown.rows),
      configured: configured(),
      banner: banner(),
      busy,
      blocked: !busy && !!running(),
      // "": a new conversation (the window says so).
      sessionTitle: active?.title ?? "",
      // The provider by id (the window names the custom ones) and its label, the model.
      providerId: provider().id,
      providerName: provider().label,
      modelText: configured() ? model() : "",
      hasFolder: !!active,
      canRetry: !busy && !running() && last?.kind === "error" && !!shown.lastPrompt,
      draft,
    };
  });

  const stopWatchdog = (run: Run) => {
    clearTimeout(run.watchdog);
    run.watchdog = undefined;
  };
  const feed = (run: Run) => {
    stopWatchdog(run);
    if (!run.running) return;
    run.watchdog = setTimeout(() => {
      if (run.tools > 0) return feed(run); // a tool runs (with its own time limit) or waits for the user
      run.chat.apply({ kind: "settled", aborted: true });
      run.chat.error("timeout", "timeout", [QUIET_TIMEOUT_MS / 1000]);
      run.worker.abort();
      finish(run);
    }, QUIET_TIMEOUT_MS);
  };
  const finish = (run: Run) => {
    stopWatchdog(run);
    if (!run.running) return;
    run.running = false;
    run.meta.running = false;
    run.meta.updated = Date.now();
    sessions.save(run.meta).catch((error) => console.error(`meta.json: ${error}`));
    later();
  };

  const startWorker = async (meta: Meta, chat: Chat): Promise<Run> => {
    const p = provider();
    // configured() made sure the key is in memory (or that none was ever saved).
    const key = await keyFor(p.id);
    const directory = sessions.directory(meta.id);
    // Scratch packages an earlier check could not remove (see Toolchain.check).
    await toolchain.sweep(pathsOf(meta.id));
    const run: Run = {
      meta,
      chat,
      running: false,
      tools: 0,
      generation,
      worker: new AgentWorker(
        {
          sessionDir: directory,
          workDir: sessions.work(meta.id),
          sessionFile: meta.file ? path.join(directory, meta.file) : null,
          provider: p.id,
          modelId: model(),
          baseUrl: p.api ? settings.value.baseUrls[p.id] : null,
          api: p.api,
          key,
          thinking: thinking(),
          reasoning: reasoning(),
          systemPrompt: systemPrompt({
            sdk: toolchain.sdk,
            sdkVersion: toolchain.sdkVersion,
            owner: owner(),
            delivery: true,
            locale: plugin.i18n.locale,
          }),
          docsRoots: toolchain.docsRoots(),
          pluginTools: [...BUILD_TOOLS, ...DELIVERY_TOOLS],
        },
        {
          tool: (call, name, args, signal) => {
            run.tools++;
            runTool(toolchain, pathsOf(meta.id), owner(), name, args, signal, delivery)
              .then((reply) => run.worker.reply(call, reply))
              .finally(() => {
                // A tool that finished after its session was deleted may have written there again.
                if (sessions.isRemoved(meta.id)) sessions.remove(meta.id).catch(() => {});
                run.tools--;
                feed(run);
                if (active?.id === meta.id) refreshProjects();
              });
          },
          event: (event) => {
            run.chat.apply(event);
            if (event.kind === "settled" && active?.id === meta.id) refreshProjects();
            if (event.kind === "settled") finish(run);
            else feed(run);
            later();
          },
          file: (file) => {
            const name = file.split(/[\\/]/).at(-1)!;
            if (run.meta.file !== name) {
              run.meta.file = name;
              sessions.save(run.meta).catch(() => {});
            }
          },
          failed: (message) => {
            if (run.running) {
              run.chat.endTurn();
              run.chat.error(message);
              finish(run);
            }
            later();
          },
          exited: (code) => {
            if (runs.get(meta.id) === run) runs.delete(meta.id);
            if (run.running) {
              run.chat.endTurn();
              run.chat.error(run.worker.lastError() || `exit ${code}`, "exited");
              finish(run);
            }
            later();
          },
        },
      ),
    };
    runs.set(meta.id, run);
    return run;
  };

  /** Ends the workers that are not generating (another session shown, settings changed). */
  const releaseIdle = (except?: string) => {
    for (const [id, run] of runs)
      if (!run.running && id !== except) {
        runs.delete(id);
        run.worker.stop().catch(() => {});
      }
  };

  const send = async (text: string) => {
    const message = text.trim();
    if (!message) return;
    // The window offers no Send then; a call anyway fails (developer messages).
    if (!configured()) throw Error("Set up a provider and its API key in the settings first");
    if (running()) throw Error("Another conversation is generating; wait a moment");
    if (!active) {
      active = await sessions.create(titleFrom(message));
      chats.set(active.id, shown);
      settings.save({ ...settings.value, lastSession: active.id });
    }
    const meta = active;
    const chat = shown;
    draft = "";
    aiWindow.set({ draft: "" }); // clears the field even if the user typed the same text again
    chat.user(message);
    chat.model = model();
    chat.begin();
    let run = runs.get(meta.id);
    if (run && !run.worker.exited && run.generation !== generation) {
      // Started with other settings (thinking, model, key): a new worker continues the session file.
      runs.delete(meta.id);
      await run.worker.stop();
      run = undefined;
    }
    if (!run || run.worker.exited) run = await startWorker(meta, chat);
    // Deleted while the worker started: nothing more for it to do.
    if (sessions.isRemoved(meta.id)) {
      runs.delete(meta.id);
      await run.worker.stop();
      return;
    }
    run.running = true;
    meta.running = true;
    meta.updated = Date.now();
    await sessions.save(meta);
    if (sessions.isRemoved(meta.id)) return;
    metas = [meta, ...metas.filter((m) => m.id !== meta.id)];
    feed(run);
    run.worker.prompt(message).catch(() => {}); // start failures arrive through `failed`
  };

  const select = async (meta: Meta | null) => {
    active = meta;
    shown = meta ? await chatOf(meta) : new Chat();
    await docs.sync(shown.rows);
    releaseIdle(meta?.id);
    projects = [];
    chosen = null;
    delivering.say("");
    refreshProjects();
    settings.save({ ...settings.value, lastSession: meta?.id ?? null });
  };

  aiWindow.on("send", send);
  aiWindow.on("draftChanged", (text: string) => {
    draft = text;
  });
  aiWindow.on("stop", () => {
    const run = active && runs.get(active.id);
    if (run?.running) run.worker.abort();
  });
  aiWindow.on("retry", () => send(shown.lastPrompt));
  aiWindow.on("newChat", () => select(null));
  aiWindow.on("selectSession", (id: string) => select(metas.find((m) => m.id === id) ?? null));

  /**
   * Deletes a conversation after the user confirmed: its record, its plugin
   * projects and their packed files (sessions/<sid>/), never what was
   * exported elsewhere. A run in it is stopped first; plugins installed from
   * it stay installed (the player owns them, the dialog says so). The open
   * conversation gives way to a new, empty one.
   */
  const removeSession = async (id: string) => {
    const meta = metas.find((m) => m.id === id);
    if (!meta) return;
    // What this session installed (recorded at each install) and is still installed from the Workshop.
    let installed: string[] = [];
    if (meta.installed?.length)
      try {
        const mine = new Set(meta.installed);
        installed = (await plugin.packages.list()).filter((p) => mine.has(p.id) && p.source?.kind === "workshop").map((p) => p.id);
      } catch (error) {
        console.error(`installed plugins: ${error}`);
      }
    const answer = await plugin.dialogs.confirm({
      message: meta.title ? "Delete the conversation “{}”?" : "Delete this conversation?",
      detail: installed.length
        ? "Its messages and the plugin projects made in it (source folders and packed .vxp files) are deleted; files you exported are kept. Plugins installed from it stay installed in the player ({1}); remove them in the plugin manager if you no longer want them."
        : "Its messages and the plugin projects made in it (source folders and packed .vxp files) are deleted; files you exported are kept.",
      confirm: "Delete",
      destructive: true,
      args: [meta.title, plugin.i18n.list(installed)],
    });
    if (!answer.confirmed || !metas.includes(meta)) return;
    // Before any await: a send whose worker is still starting sees it and bails.
    sessions.markRemoved(id);
    const run = runs.get(id);
    if (run) {
      runs.delete(id);
      stopWatchdog(run);
      if (run.running) {
        run.running = false;
        run.worker.abort();
      }
      await run.worker.stop();
    }
    metas = metas.filter((m) => m !== meta);
    chats.delete(id);
    if (active?.id === id) await select(null);
    plugin.update();
    try {
      await sessions.remove(id);
    } catch (error) {
      console.error(`delete ${id}: ${error}`);
      // Said in the conversation shown; a session whose meta.json is still there stays listed.
      shown.notice("chat-delete-failed", [meta.title, (error as Error).message.split("\n")[0]]);
      if (await sessions.restore(meta)) metas = [...metas, meta].sort((a, b) => b.updated - a.updated);
      await docs.sync(shown.rows);
      plugin.update();
    }
  };
  aiWindow.on("deleteSession", (id: string) => removeSession(id));
  aiWindow.on("deleteChat", () => active && removeSession(active.id));
  aiWindow.on("toggle", (id: string) => shown.toggle(id));
  aiWindow.on("copy", (id: string) => {
    const row = shown.rows.find((r) => r.id === id);
    if (row) return plugin.clipboard.writeText(cut(row.text, 1000 * 1000));
  });
  aiWindow.on("copyCode", (code: string) => plugin.clipboard.writeText(cut(code, 1000 * 1000)));
  aiWindow.on("openLink", (url: string) => {
    // Only web links; the player refuses other schemes too.
    if (/^https?:\/\//i.test(url)) return plugin.system.openUrl(url);
  });
  aiWindow.on("openSettings", () => plugin.settings.show("byok"));
  aiWindow.on("openFolder", () => active && plugin.system.openPath(sessions.work(active.id)));
  aiWindow.on("chooseProject", (index: number) => {
    chosen = projects[index]?.id ?? chosen;
    delivering.say("");
  });
  aiWindow.on("install", action((meta, p) => installFrom(pathsOf(meta.id), p.id, false)));
  aiWindow.on(
    "exportVxp",
    action(async (meta, p) => {
      exporting = true;
      try {
        cardSay("packing", [p.id]);
        const packed = await toolchain.pack(pathsOf(meta.id), p.id, owner());
        if (!packed.ok || !packed.file) return cardSay(packed.say?.code ?? "failed", packed.say?.args ?? [packed.text.split("\n")[0]]);
        const target = await plugin.dialogs.saveFile({ title: "Export Plugin Package", fileName: path.basename(packed.file) });
        if (!target) return cardSay("");
        await fs.copyFile(packed.file, target);
        cardSay("exported", [path.basename(target)]);
      } finally {
        exporting = false;
      }
    }),
  );
  aiWindow.on(
    "exportSource",
    action(async (meta, p) => {
      exporting = true;
      try {
        const { data, fileName, local } = await toolchain.exportSource(pathsOf(meta.id), p.id);
        const target = await plugin.dialogs.saveFile({ title: "Export Source (Ready to Use as a GitHub Repository)", fileName });
        if (!target) return cardSay("");
        await fs.writeFile(target, data);
        // What to change before publishing: the LICENSE placeholder, and a local. id.
        cardSay(local ? "exported-source-local" : "exported-source", [path.basename(target)]);
      } finally {
        exporting = false;
      }
    }),
  );
  aiWindow.on("openProject", action((_meta, p) => plugin.system.openPath(p.directory)));
  aiWindow.on("openPi", () => plugin.system.openUrl(PI_URL));

  // ---- settings page (Options ▸ Plugins ▸ Vestra Player AI)
  /** A provider list entry: pi's providers by their own name (data), the custom endpoints in words. */
  const entry = (x: Provider) =>
    x.id === "custom-openai"
      ? plugin.i18n.text("Custom · OpenAI format")
      : x.id === "custom-anthropic"
        ? plugin.i18n.text("Custom · Anthropic format")
        : x.label;
  const page = plugin.settings.page("settings.slint", { id: "byok", category: "plugins", title: "Vestra Player AI" });
  page.bind(() => {
    const p = provider();
    const key = keys.get(p.id) ?? "";
    const models = modelsOf(p);
    return {
      // A provider no longer offered shows as the first entry, so the list
      // does not pretend another one is selected.
      providers: [...(gone() ? [plugin.i18n.text("{} (no longer offered)", [goneName(p.id)])] : []), ...providers.map(entry)],
      providerIndex: gone() ? 0 : providers.indexOf(p),
      custom: !!p.api,
      canList: !gone() && (!!p.api || !!p.list),
      baseUrl: settings.value.baseUrls[p.id] ?? "",
      keyText,
      keyState: gone()
        ? text("gone-key")
        : reading.has(p.id)
          ? text("reading")
          : !keys.has(p.id)
            ? keyError
              ? text("unreadable")
              : text("not-read")
            : p.api && !key
              ? text("local-empty")
              : masked(key),
      hasKey: !!key,
      models,
      modelIndex: models.indexOf(model()),
      modelText: model(),
      listing,
      // The name of a saved provider no longer offered ("" otherwise).
      gone: gone() ? goneName(p.id) : "",
      message: pageMessage,
      messageWarn: pageWarn,
      ownerText: settings.value.owner ?? "",
      thinkingIndex: THINKING.indexOf(thinking()),
      reasons: reasoning() ?? false,
      ownerHint: ownerHint(),
    };
  });
  /** What the owner field means now. */
  const ownerHint = (): Say => {
    try {
      const name = checkOwner(settings.value.owner ?? "");
      return name ? text("owner", [name]) : text("owner-none");
    } catch (error) {
      return text(error instanceof OwnerError ? error.code : "owner-chars");
    }
  };
  page.on("setOwner", (text: string) => {
    // Saved as typed; checkOwner decides what is used (the hint says why not).
    settings.save({ ...settings.value, owner: text.trim() });
    changed(); // the next message gets a system prompt with the new owner
  });
  /** The page's message line: a template key of settings.slint and its arguments. */
  const say = (code: string, args: (string | number)[] = [], warn = false, n = 0) => {
    pageMessage = text(code, args, n);
    pageWarn = warn;
  };
  /** A change of service, key or model applies to the next message. */
  const changed = () => {
    generation++;
    releaseIdle();
  };
  /** Pulls the lists back to the model values (a one-way index stops following after a pick). */
  const syncIndexes = () => {
    const p = provider();
    page.set({ providerIndex: gone() ? 0 : providers.indexOf(p), modelIndex: modelsOf(p).indexOf(model()) });
  };
  page.on("chooseProvider", async (index: number) => {
    const p = providers[gone() ? index - 1 : index];
    if (!p) return;
    // The model shown is the model used: a provider chosen for the first
    // time gets its default model saved at once.
    const models = settings.value.models[p.id] ? settings.value.models : { ...settings.value.models, [p.id]: p.defaultModel };
    settings.save({ ...settings.value, provider: p.id, models });
    keyText = "";
    keyError = null;
    say("");
    changed();
    syncIndexes();
    // pi's catalog can be out of date: ask the provider when its key is at hand.
    keyFor(p.id)
      .then((key) => (key && p.list && !listed.has(p.id) ? fetchModels(false) : undefined))
      .catch(() => {}); // a keychain failure shows through keyError
  });
  page.on("saveKey", async (text: string) => {
    const key = text.trim();
    if (!key) return;
    if (gone()) {
      say("need-provider", [], true);
      return;
    }
    const p = provider();
    try {
      await plugin.secrets.set(secretName(p.id), key);
    } catch (error) {
      say("save-failed", [(error as Error).message], true);
      return;
    }
    keys.set(p.id, key);
    markSaved(p.id, true);
    keyError = null;
    keyText = "";
    page.set({ keyText: "" });
    say("key-saved", [p.id, p.label]);
    changed();
    if ((p.api && settings.value.baseUrls[p.id]) || p.list) await fetchModels(false);
  });
  /** The key of a provider that is no longer offered: kept until the user deletes it. */
  page.on("deleteOldKey", async () => {
    if (!gone()) return;
    const id = settings.value.provider;
    try {
      await plugin.secrets.delete(secretName(id));
    } catch (error) {
      say("delete-failed", [(error as Error).message], true);
      return;
    }
    markSaved(id, false);
    say("old-key-deleted");
  });
  page.on("deleteKey", async () => {
    const p = provider();
    try {
      await plugin.secrets.delete(secretName(p.id));
    } catch (error) {
      say("delete-failed", [(error as Error).message], true);
      return;
    }
    keys.set(p.id, "");
    markSaved(p.id, false);
    keyError = null;
    say("key-deleted", [p.id, p.label]);
    changed();
  });
  /** Saves a custom Base URL; `loud`: say why an invalid one is not saved. */
  const saveBaseUrl = (text: string, loud: boolean): boolean => {
    const p = provider();
    if (!p.api) return false;
    let url = "";
    try {
      url = text.trim() ? checkBaseUrl(text) : "";
    } catch (error) {
      if (loud) say(error instanceof BaseUrlError ? error.code : "url-invalid", [], true);
      return false;
    }
    // The Anthropic SDK adds /v1/messages itself.
    if (p.api === "anthropic-messages") url = url.replace(/\/v1$/, "");
    if (url !== (settings.value.baseUrls[p.id] ?? "")) {
      settings.save({ ...settings.value, baseUrls: { ...settings.value.baseUrls, [p.id]: url } });
      listed.delete(p.id);
      changed();
    }
    if (loud) say("");
    return !!url;
  };
  /** Lists a custom endpoint's models; on failure the model id is typed by hand. */
  /**
   * Asks the provider for its models: a custom endpoint at its Base URL, a
   * built-in one where its API lists them. Listed models come first, then
   * the catalog's others; a current model that is not listed is replaced.
   */
  const fetchModels = async (loud: boolean) => {
    const p = provider();
    if (gone()) return;
    const base = settings.value.baseUrls[p.id];
    const spec = p.api ? (base ? customList(p, base) : undefined) : p.list;
    if (!spec) {
      if (loud) say(p.api ? "need-url" : "no-list", [], true);
      return;
    }
    listing = true;
    plugin.update();
    try {
      const models = await listModels(spec, await keyFor(p.id).catch(() => ""));
      if (provider().id !== p.id) return; // another provider was chosen meanwhile
      listed.set(p.id, models);
      const before = model();
      const chosen = pickListed(p, before, models);
      settings.save({ ...settings.value, models: { ...settings.value.models, [p.id]: chosen } });
      if (chosen === before) say("fetched", [], false, models.length);
      else say("fetched-replaced", [before, chosen], false, models.length);
      changed();
    } catch (error) {
      listed.delete(p.id);
      if (loud || p.api) {
        if (error instanceof NoModelsError) say(p.api ? "fetch-empty-custom" : "fetch-empty", [], true);
        else say(p.api ? "fetch-failed-custom" : "fetch-failed", [(error as Error).message], true);
      }
    } finally {
      listing = false;
      syncIndexes();
      plugin.update();
    }
  };
  page.on("setBaseUrl", (text: string) => {
    saveBaseUrl(text, false);
  });
  page.on("acceptBaseUrl", async (text: string) => {
    if (saveBaseUrl(text, true)) await fetchModels(true);
  });
  page.on("chooseModel", (id: string) => {
    settings.save({ ...settings.value, models: { ...settings.value.models, [provider().id]: id.trim() } });
    changed();
  });
  page.on("fetchModels", () => fetchModels(true));
  page.on("chooseThinking", (index: number) => {
    const level = THINKING[index];
    if (!level) return;
    settings.save({ ...settings.value, thinking: level });
    changed();
  });
  page.on("setReasons", (on: boolean) => {
    const p = provider();
    if (!p.api) return;
    const saved = settings.value.reasoning && typeof settings.value.reasoning === "object" ? settings.value.reasoning : {};
    settings.save({ ...settings.value, reasoning: { ...saved, [reasoningKey(p.id, model())]: on } });
    changed();
  });

  plugin.onDispose(async () => {
    clearTimeout(refresh);
    // Runs still in progress stay marked in meta.json: the next start says so.
    await Promise.all([...runs.values()].map((run) => (stopWatchdog(run), run.worker.stop(300))));
  });

  refreshProjects();

  plugin.commands.add({
    id: "workshop.open",
    path: ["Vestra Player AI…"],
    order: 820,
    group: "windows",
    // The bottom bar's button (player 0.5.10), left of the Plugin Store's (200).
    button: { icon: "ai", order: 100 },
    run: () => {
      loadKey(); // not awaited: a keychain dialog must not hold the window
      return aiWindow.show();
    },
  });
}
