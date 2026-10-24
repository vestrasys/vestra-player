// Installing a session's plugin in the player (plugins.manage): pack →
// packages.validateArchive (the player's checks, Slint included) → an
// install the user confirmed, reversible with Roll Back (rollback, or
// uninstall after a first install). Used by the agent's install_plugin and by the
// result card's buttons. After an install, bindings the player ignored
// because another command holds the key (bindings.list, `shadowed`) are
// reported to the user and the agent.
import type { PackageCandidate, PackageChange, Plugin, PluginError } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import type { SessionPaths, ToolResult, Toolchain } from "./toolchain.ts";

/** What the model reads for the end state of an install, rollback or uninstall. */
const CHANGE_TEXT: Record<string, string> = {
  committed: "done",
  cancelled: "cancelled",
  failed: "failed",
  failedDegraded: "failed, the previous version could not be fully restored",
  tooLate: "too late to cancel",
};

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const code = (error: unknown) => (error as PluginError)?.code;

/** A binding of the installed plugin that another command holds (the player ignores it). */
export type Shadowed = {
  /** The input, e.g. "]". */
  input: string;
  /** The installed plugin's command. */
  title: string;
  /** The holder: "host" (the player) or a plugin id, and its command's title. */
  plugin: string;
  holder: string;
};

/**
 * What the card says: a template key of ai.slint and its arguments (data).
 * Installs, rollbacks and uninstalls (`progress`, `result`): args are
 * [action, a, b, status, message] with action "install" (a name, b version),
 * "rollback" (a version) or "uninstall" (a id).
 */
export type CardSay = {
  code: string;
  args: string[];
};

type Action = [action: "install" | "rollback" | "uninstall", a: string, b: string];

export class Delivering {
  private readonly plugin: Plugin;
  private readonly toolchain: Toolchain;
  private readonly owner: () => string;
  /** Ids installed during this run: Roll Back is offered for them. */
  readonly installed = new Set<string>();
  /** The result card's line: what the last action did. */
  status: CardSay = { code: "", args: [] };
  /** After an install: the bindings that did not take effect (listed under the line). */
  shadowedKeys: Shadowed[] = [];
  busy = false;

  constructor(plugin: Plugin, toolchain: Toolchain, owner: () => string) {
    this.plugin = plugin;
    this.toolchain = toolchain;
    this.owner = owner;
  }

  say(code: string, args: (string | number)[] = [], keys: Shadowed[] = []) {
    this.status = { code, args: args.map(String) };
    this.shadowedKeys = keys;
    this.plugin.update();
  }

  /** One action at a time; the card's buttons are disabled meanwhile. */
  private async exclusive(work: () => Promise<ToolResult>): Promise<ToolResult> {
    if (this.busy) return { ok: false, text: "Another install or rollback is in progress; wait a moment" };
    this.busy = true;
    this.plugin.update();
    try {
      return await work();
    } finally {
      this.busy = false;
      this.plugin.update();
    }
  }

  /** Packs the project and lets the player validate the package. */
  private async candidate(session: SessionPaths, id: string, signal?: AbortSignal): Promise<{ candidate?: PackageCandidate; failure?: ToolResult }> {
    this.say("packing", [id]);
    const packed = await this.toolchain.pack(session, id, this.owner(), signal);
    if (!packed.ok || !packed.file) return { failure: packed };
    this.say("validating", [id]);
    try {
      return { candidate: await this.plugin.packages.validateArchive(packed.file) };
    } catch (error) {
      // Manifest, package and Slint problems the player found.
      const kind = code(error) ?? "error";
      return {
        failure: { ok: false, text: `The player's validation failed (${kind}): ${errorText(error)}`, say: { code: "invalid", args: [kind, errorText(error)] } },
      };
    }
  }

  /**
   * Inputs of `id`'s commands that another command holds: the player
   * ignores them silently (its log only). Read once the plugin has left
   * "starting"; [] when it is not running or the list is unavailable.
   */
  private async shadowed(id: string): Promise<Shadowed[]> {
    try {
      for (let wait = 0; wait < 25; wait++) {
        const entry = (await this.plugin.packages.list()).find((p) => p.id === id);
        if (entry?.state !== "starting") {
          if (entry?.state !== "running") return [];
          break;
        }
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
      const { commands } = await this.plugin.bindings.list();
      const holder = (ref: { plugin: string; command: string }) =>
        commands.find((c) => c.plugin === ref.plugin && c.command === ref.command)?.title || ref.command;
      return commands
        .filter((c) => c.plugin === id)
        .flatMap((c) => c.shadowed.map((s) => ({ input: s.input, title: c.title, plugin: s.by.plugin, holder: holder(s.by) })));
    } catch {
      return [];
    }
  }

  /** Follows an install or rollback to its end; the card shows each status. */
  private async follow(change: PackageChange, action: Action): Promise<{ ok: boolean; text: string; status: string }> {
    const listening = change.onChange((status) => this.say("progress", [...action, status]));
    try {
      const status = await change.done;
      const [kind, a, b] = action;
      const what = kind === "install" ? `Install ${a} ${b}` : kind === "rollback" ? `Roll back to ${a}` : `Uninstall ${a}`;
      const text = `${what}: ${CHANGE_TEXT[status] ?? status}${change.message ? ` (${change.message})` : ""}`;
      this.status = { code: "result", args: [...action, status, change.message ?? ""] };
      return { ok: status === "committed", text, status };
    } finally {
      await listening.dispose();
    }
  }

  /**
   * Installs the project. `ask`: the agent asked, so the user confirms in a
   * dialog first (a click on Install is the user's own decision). Another
   * source's plugin of the same id is replaced only after asking.
   */
  install(session: SessionPaths, id: string, ask: boolean, signal?: AbortSignal): Promise<ToolResult> {
    return this.exclusive(async () => {
      const directory = await this.toolchain.project(session, id);
      const manifest = JSON.parse(await fs.readFile(path.join(directory, "manifest.json"), "utf8"));
      const name = String(manifest.name ?? id);
      const version = String(manifest.version);
      if (ask) {
        const capabilities: string[] = manifest.capabilities ?? [];
        const answer = await this.plugin.dialogs.confirm({
          message: "Install the AI-generated plugin “{}”?",
          detail: capabilities.length
            ? "{1} {2}\nCapabilities: {3}\nTo undo the install, click Roll Back (Undo Install) in the AI window."
            : "{1} {2}\nCapabilities: none\nTo undo the install, click Roll Back (Undo Install) in the AI window.",
          confirm: "Install",
          // {0} (the message's {}) = name, {1} = id, {2} = version, {3} = capabilities.
          args: [name, id, version, capabilities.join(", ")],
        });
        if (!answer.confirmed) {
          this.say("install-cancelled");
          return { ok: false, text: "The user cancelled the install" };
        }
      }
      const { candidate, failure } = await this.candidate(session, id, signal);
      if (!candidate) {
        this.say(failure!.say?.code ?? "failed", failure!.say?.args ?? [failure!.text.split("\n")[0]]);
        return failure!;
      }
      this.say("installing", [name]);
      const options = { source: { kind: "workshop" as const } };
      let change: PackageChange;
      try {
        change = await candidate.install(options);
      } catch (error) {
        if (code(error) === "unsupported") {
          this.say("builtin-id", [id]);
          return { ok: false, text: `${id} has the name of a plugin built into the player and cannot be installed; choose another id.` };
        }
        if (code(error) !== "conflict") throw error;
        const answer = await this.plugin.dialogs.confirm({
          message: "Replace the installed {}?",
          detail:
            "It was installed from elsewhere (not made by Vestra Player AI). After the replacement the new plugin starts with a new, empty data folder; the original plugin's data is kept and is used again when you reinstall it from its original source.",
          confirm: "Replace",
          destructive: true,
          args: [id],
        });
        if (!answer.confirmed) {
          this.say("kept-installed");
          return { ok: false, text: `The user did not replace ${id}, which was installed from elsewhere` };
        }
        change = await candidate.install({ ...options, replace: true });
      }
      const { ok, text } = await this.follow(change, ["install", name, version]);
      if (!ok) {
        this.plugin.update();
        return { ok, text };
      }
      this.installed.add(id);
      const shadowed = await this.shadowed(id);
      this.say("installed", [name, version], shadowed);
      const keys = shadowed.map((s) => `${s.input} (${s.title}) is taken by ${s.plugin === "host" ? "the player" : s.plugin} "${s.holder}"`);
      return {
        ok,
        text:
          `${text}. The plugin is enabled in the player; the user can click Roll Back (Undo Install) in the AI window to return to the state before the install.` +
          (keys.length
            ? `\nShortcuts not in effect: ${keys.join("; ")}. The player ignored these keys (on a conflict the user's settings come first, then the player and the plugins loaded earlier). Tell the user and suggest keys that are free, then install again.`
            : ""),
      };
    });
  }

  /**
   * Undoes this run's install: back to the previous version, or, for a
   * first install, removes the plugin again (its data stays, so a later
   * install from here reconnects to it).
   */
  rollback(id: string): Promise<ToolResult> {
    return this.exclusive(async () => {
      const entry = (await this.plugin.packages.list()).find((p) => p.id === id && p.origin === "installed");
      if (!entry) {
        this.installed.delete(id);
        this.say("not-installed", [id]);
        return { ok: false, text: `${id} is not installed` };
      }
      this.say("rolling-back", [id]);
      const change = entry.previousVersion ? await this.plugin.packages.rollback(id) : await this.plugin.packages.uninstall(id);
      const { ok, text } = await this.follow(change, entry.previousVersion ? ["rollback", entry.previousVersion, ""] : ["uninstall", id, ""]);
      if (ok) this.installed.delete(id);
      this.plugin.update();
      return { ok, text };
    });
  }
}
