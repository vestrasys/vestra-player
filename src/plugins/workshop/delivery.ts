// Installing a session's plugin in the player (plugins.manage): pack →
// packages.validateArchive (the player's checks, Slint included) → an
// install the user confirmed. Used by the agent's install_plugin and by the
// result card's buttons. After an install, bindings the player ignored
// because another command holds the key (bindings.list, `shadowed`) are
// reported to the user and the agent.
import type { PackageCandidate, PackageChange, Plugin, PluginError } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { PLAYER_NEXT, playerCode, playerMessage, type SessionPaths, type ToolResult, type Toolchain } from "./toolchain.ts";

/** What the model reads for the end state of an install. */
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
 * Installs (`progress`, `result`): args are [action, a, b, status, message]
 * with action "install" (a name, b version).
 */
export type CardSay = {
  code: string;
  args: string[];
};

type Action = [action: "install", a: string, b: string];

export class Delivering {
  private readonly plugin: Plugin;
  private readonly toolchain: Toolchain;
  private readonly owner: () => string;
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
    if (this.busy) return { ok: false, text: "Another install is in progress; wait a moment" };
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
      // Manifest, package and Slint problems the player found (check runs the
      // same validation; this stays as the last line of defence).
      const kind = playerCode(error);
      return {
        failure: {
          ok: false,
          text: `The player's validation failed (${kind}): ${playerMessage(error)}\n${PLAYER_NEXT}, before installing again.`,
          say: { code: "invalid", args: [kind, errorText(error)] },
        },
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

  /** Follows an install to its end; the card shows each status. */
  private async follow(change: PackageChange, action: Action): Promise<{ ok: boolean; text: string; status: string }> {
    const listening = change.onChange((status) => this.say("progress", [...action, status]));
    try {
      const status = await change.done;
      const [, a, b] = action;
      const what = `Install ${a} ${b}`;
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
          detail: capabilities.length ? "{1} {2}\nCapabilities: {3}" : "{1} {2}\nCapabilities: none",
          confirm: "Install",
          // {0} (the message's {}) = name, {1} = id, {2} = version, {3} = capabilities.
          args: [name, id, version, this.plugin.i18n.list(capabilities)],
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
      const shadowed = await this.shadowed(id);
      this.say("installed", [name, version], shadowed);
      const keys = shadowed.map((s) => `${s.input} (${s.title}) is taken by ${s.plugin === "host" ? "the player" : s.plugin} "${s.holder}"`);
      return {
        ok,
        text:
          `${text}. The plugin is enabled in the player.` +
          (keys.length
            ? `\nShortcuts not in effect: ${keys.join("; ")}. The player ignored these keys (on a conflict the user's settings come first, then the player and the plugins loaded earlier). Tell the user and suggest keys that are free, then install again.`
            : ""),
      };
    });
  }
}
