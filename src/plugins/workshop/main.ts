import type { Plugin } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { AgentWorker } from "./agent.ts";
import { Delivering } from "./delivery.ts";
import { systemPrompt } from "./prompt.ts";
import { Toolchain, checkOwner, type Project, type SessionPaths } from "./toolchain.ts";
import { BUILD_TOOLS, DELIVERY_TOOLS, runTool } from "./toolhost.ts";
import { Chat } from "./chat.ts";
import { MessageDocs, cut } from "./messages.ts";
import {
  DEFAULT_SETTINGS,
  LEGACY_COMPAT,
  checkBaseUrl,
  customList,
  goneName,
  goneNotice,
  listModels,
  loadProviders,
  masked,
  migrate,
  pickListed,
  secretName,
  validSettings,
  type AiSettings,
  type Provider,
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
}

/** Vestra Player AI: chat with a model of the user's choice (BYOK); the agent works in the session folder. */
export default async function activate(plugin: Plugin) {
  const providers = loadProviders(import.meta.dirname);
  const sessions = new Sessions(plugin.dataDirectory);
  const toolchain = new Toolchain(import.meta.dirname, plugin.dataDirectory);
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
  const delivery = {
    install: (session: SessionPaths, id: string, signal: AbortSignal) => delivering.install(session, id, true, signal),
  };
  /**
   * provider → key, read from the keychain once, on demand. Never during
   * activation: a read can wait on the system's authorization dialog
   * (macOS asks again after the player was re-signed), and activation must
   * not wait on the user.
   */
  const keys = new Map<string, string>();
  const reading = new Map<string, Promise<string>>();
  /** The keychain read failed or was denied (cleared by the next attempt or a save). */
  let keyError = "";
  const runs = new Map<string, Run>(); // session id → its worker
  const chats = new Map<string, Chat>(); // session id → rows (loaded once)

  let metas: Meta[] = await sessions.list();
  let active: Meta | null = metas.find((m) => m.id === settings.value.lastSession) ?? null;
  let draft = "";
  let pageMessage = "";
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
  const KEY_UNREADABLE = "无法读取钥匙串中的 key，请在设置中重新保存。";
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
            throw new Error("钥匙串里没有这个 key");
          }
          keys.set(id, key);
          keyError = "";
          return key;
        } catch (error) {
          keyError = `${KEY_UNREADABLE}（${(error as Error).message}）`;
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
  /** The line above the conversation: keychain state or missing setup. */
  const banner = () => {
    const p = provider();
    if (gone()) return goneNotice(p.id);
    if (reading.has(p.id)) return "正在读取钥匙串中的 API key…（系统可能会请求授权）";
    if (keyError) return keyError;
    if (!configured()) return !keys.has(p.id) ? "尚未读取 API key。" : "尚未配置服务商：请先在设置中选择服务商并保存 API key。";
    return "";
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
        chat.notice("上次的生成被中断（播放器或插件已退出）");
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
  const cardSay = (text: string) => {
    delivering.status = text;
    plugin.update();
  };
  /** A card action: failures show on the card's line. */
  const action = (work: (meta: Meta, p: Project) => Promise<unknown>) => async () => {
    const p = project();
    if (!active || !p) return;
    try {
      await work(active, p);
    } catch (error) {
      cardSay(`出错了：${(error as Error).message}`);
    } finally {
      refreshProjects();
    }
  };

  const window = plugin.windows.create("ai.slint");
  window.bind(() => {
    const busy = !!(active && runs.get(active.id)?.running);
    const last = shown.rows.at(-1);
    const p = project();
    return {
      hasProject: !!p,
      projects: projects.map((x) => `${x.name} · ${x.id}`),
      projectIndex: Math.max(0, projects.findIndex((x) => x.id === chosen)),
      projectTitle: p ? `${p.name} · ${p.id} ${p.version}` : "",
      cardStatus: delivering.status,
      cardBusy: delivering.busy || exporting,
      canRollback: !!p && delivering.installed.has(p.id),
      sessions: sessionRows(),
      rows: docs.props(shown.rows),
      configured: configured(),
      banner: banner(),
      busy,
      blocked: !busy && !!running(),
      sessionTitle: active?.title ?? "新对话",
      modelText: configured() ? `${provider().label} · ${model()}` : "",
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
      run.chat.error("timeout", `超过 ${QUIET_TIMEOUT_MS / 1000} 秒没有收到服务商的回复，已停止。`);
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
    const run: Run = {
      meta,
      chat,
      running: false,
      tools: 0,
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
          systemPrompt: systemPrompt({ sdk: toolchain.sdk, index: toolchain.docsIndex(), sdkVersion: toolchain.sdkVersion, owner: owner(), delivery: true }),
          docsRoots: toolchain.docsRoots(),
          pluginTools: [...BUILD_TOOLS, ...DELIVERY_TOOLS],
        },
        {
          tool: (call, name, args, signal) => {
            run.tools++;
            runTool(toolchain, pathsOf(meta.id), owner(), name, args, signal, delivery)
              .then((reply) => run.worker.reply(call, reply))
              .finally(() => {
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
              run.chat.error(run.worker.lastError() || `exit ${code}`, "agent 进程意外退出。");
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
    if (!configured()) throw Error("请先在设置中配置服务商和 API key");
    if (running()) throw Error("另一会话正在生成，请稍候");
    if (!active) {
      active = await sessions.create(titleFrom(message));
      chats.set(active.id, shown);
      settings.save({ ...settings.value, lastSession: active.id });
    }
    const meta = active;
    const chat = shown;
    draft = "";
    window.set({ draft: "" }); // clears the field even if the user typed the same text again
    chat.user(message);
    chat.model = model();
    chat.begin();
    let run = runs.get(meta.id);
    if (!run || run.worker.exited) run = await startWorker(meta, chat);
    run.running = true;
    meta.running = true;
    meta.updated = Date.now();
    await sessions.save(meta);
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
    delivering.status = "";
    refreshProjects();
    settings.save({ ...settings.value, lastSession: meta?.id ?? null });
  };

  window.on("send", send);
  window.on("draftChanged", (text: string) => {
    draft = text;
  });
  window.on("stop", () => {
    const run = active && runs.get(active.id);
    if (run?.running) run.worker.abort();
  });
  window.on("retry", () => send(shown.lastPrompt));
  window.on("newChat", () => select(null));
  window.on("selectSession", (id: string) => select(metas.find((m) => m.id === id) ?? null));
  window.on("toggle", (id: string) => shown.toggle(id));
  window.on("copy", (id: string) => {
    const row = shown.rows.find((r) => r.id === id);
    if (row) return plugin.clipboard.writeText(cut(row.text, 1000 * 1000, ""));
  });
  window.on("copyCode", (code: string) => plugin.clipboard.writeText(cut(code, 1000 * 1000, "")));
  window.on("openLink", (url: string) => {
    // Only web links; the player refuses other schemes too.
    if (/^https?:\/\//i.test(url)) return plugin.system.openUrl(url);
  });
  window.on("openSettings", () => plugin.settings.show("byok"));
  window.on("openFolder", () => active && plugin.system.openPath(sessions.work(active.id)));
  window.on("chooseProject", (index: number) => {
    chosen = projects[index]?.id ?? chosen;
    delivering.status = "";
  });
  window.on("install", action((meta, p) => delivering.install(pathsOf(meta.id), p.id, false)));
  window.on("rollback", action((_meta, p) => delivering.rollback(p.id)));
  window.on(
    "exportVxp",
    action(async (meta, p) => {
      exporting = true;
      try {
        cardSay(`正在打包 ${p.id}…`);
        const packed = await toolchain.pack(pathsOf(meta.id), p.id, owner());
        if (!packed.ok || !packed.file) return cardSay(packed.text.split("\n")[0]);
        const target = await plugin.dialogs.saveFile({ title: "导出插件包", fileName: path.basename(packed.file) });
        if (!target) return cardSay("");
        await fs.copyFile(packed.file, target);
        cardSay(`已导出 ${path.basename(target)}`);
      } finally {
        exporting = false;
      }
    }),
  );
  window.on(
    "exportSource",
    action(async (meta, p) => {
      exporting = true;
      try {
        const { data, fileName, notes } = await toolchain.exportSource(pathsOf(meta.id), p.id);
        const target = await plugin.dialogs.saveFile({ title: "导出源码（可以直接作为 GitHub 仓库）", fileName });
        if (!target) return cardSay("");
        await fs.writeFile(target, data);
        cardSay(`已导出 ${path.basename(target)}。${notes.join("；")}。`);
      } finally {
        exporting = false;
      }
    }),
  );
  window.on("openProject", action((_meta, p) => plugin.system.openPath(p.directory)));
  window.on("openPi", () => plugin.system.openUrl(PI_URL));

  // ---- settings page (Options ▸ 插件 ▸ Vestra Player AI)
  const page = plugin.settings.page("settings.slint", { id: "byok", category: "plugins", title: "Vestra Player AI" });
  page.bind(() => {
    const p = provider();
    const key = keys.get(p.id) ?? "";
    const models = modelsOf(p);
    return {
      // A provider no longer offered shows as the first entry, so the list
      // does not pretend another one is selected.
      providers: [...(gone() ? [`${goneName(p.id)}（已不再提供）`] : []), ...providers.map((x) => x.label)],
      providerIndex: gone() ? 0 : providers.indexOf(p),
      custom: !!p.api,
      canList: !gone() && (!!p.api || !!p.list),
      baseUrl: settings.value.baseUrls[p.id] ?? "",
      keyText,
      keyState: gone()
        ? "旧服务商的 key 不会自动删除"
        : reading.has(p.id)
        ? "正在读取钥匙串…"
        : !keys.has(p.id)
          ? keyError
            ? "无法读取钥匙串中的 key，请重新保存"
            : "尚未读取（打开 AI 窗口时读取）"
          : p.api && !key
            ? "未保存（本地服务可以不填）"
            : masked(key),
      hasKey: !!key,
      models,
      modelIndex: models.indexOf(model()),
      modelText: model(),
      listing,
      gone: gone() ? goneNotice(p.id) : "",
      message: pageMessage,
      messageWarn: pageWarn,
      ownerText: settings.value.owner ?? "",
      ownerHint: ownerHint(),
    };
  });
  /** What the owner field means now. */
  const ownerHint = () => {
    try {
      const name = checkOwner(settings.value.owner ?? "");
      return name
        ? `新插件的 id 是 ${name}.<名字>（官方插件仓库要求 id 前缀就是源码仓库所属的 GitHub 用户或组织）。`
        : "没有填写：新插件的 id 用 local.<名字>，只适合在本机使用；要发布到官方插件仓库，请填你的 GitHub 用户名。";
    } catch (error) {
      return `${(error as Error).message}（新插件暂时用 local.<名字>）`;
    }
  };
  page.on("setOwner", (text: string) => {
    // Saved as typed; checkOwner decides what is used (the hint says why not).
    settings.save({ ...settings.value, owner: text.trim() });
    changed(); // the next message gets a system prompt with the new owner
  });
  const say = (text: string, warn = false) => {
    pageMessage = text;
    pageWarn = warn;
  };
  /** A change of service, key or model applies to the next message. */
  const changed = () => releaseIdle();
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
    keyError = "";
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
      say("请先选择一个服务商，再保存 key。", true);
      return;
    }
    const p = provider();
    try {
      await plugin.secrets.set(secretName(p.id), key);
    } catch (error) {
      say(`没能保存到钥匙串：${(error as Error).message}`, true);
      return;
    }
    keys.set(p.id, key);
    markSaved(p.id, true);
    keyError = "";
    keyText = "";
    page.set({ keyText: "" });
    say(`${p.label} 的 key 已保存。`);
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
      say(`没能从钥匙串删除：${(error as Error).message}`, true);
      return;
    }
    markSaved(id, false);
    say("旧服务商的 key 已删除。");
  });
  page.on("deleteKey", async () => {
    const p = provider();
    try {
      await plugin.secrets.delete(secretName(p.id));
    } catch (error) {
      say(`没能从钥匙串删除：${(error as Error).message}`, true);
      return;
    }
    keys.set(p.id, "");
    markSaved(p.id, false);
    keyError = "";
    say(`${p.label} 的 key 已删除。`);
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
      if (loud) say((error as Error).message, true);
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
      if (loud) say(p.api ? "请先填写 Base URL。" : "这个服务商不提供模型列表。", true);
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
      say(chosen === before ? `获取到 ${models.length} 个模型。` : `获取到 ${models.length} 个模型。「${before}」不在列表中，已改用「${chosen}」。`);
      changed();
    } catch (error) {
      listed.delete(p.id);
      if (loud || p.api)
        say(p.api ? `没能获取模型列表（${(error as Error).message}），请直接填写模型 ID。` : `没能获取模型列表（${(error as Error).message}），可以从 pi 自带的列表中选择。`, true);
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
    run: () => {
      loadKey(); // not awaited: a keychain dialog must not hold the window
      return window.show();
    },
  });
}
