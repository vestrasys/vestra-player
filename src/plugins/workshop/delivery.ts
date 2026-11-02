// Installing a session's plugin in the player (plugins.manage): pack →
// packages.validateArchive (the player's checks, Slint included) → an
// install the user confirmed. Used by the agent's install_plugin and by the
// result card's buttons. After an install, bindings the player ignored
// because another command holds the key (bindings.list, `shadowed`) are
// reported to the user and the agent, and so is the plugin's real state: an
// update of a plugin the user disabled keeps it disabled (player after
// 0.6.3), which the workshop respects and never changes.
import type { PackageCandidate, PackageChange, Plugin, PluginError, PluginInfo } from "@vestra-player/sdk";
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

/**
 * What the model reads for the plugin's state after a committed install
 * (packages.list). "disabled" and "failed" have their own texts.
 */
const STATE_TEXT: Record<string, string> = {
  running: "The plugin is enabled and running in the player.",
  starting: "The plugin is enabled and still starting in the player.",
  stopped: "The plugin is installed and enabled but not running in the player.",
  missing: "The player does not list the plugin.",
};

/** The model's text when an update kept the plugin the user disabled disabled. */
const DISABLED_TEXT =
  "The plugin is installed but disabled: the user turned it off in the player, so the new version did not start. " +
  "Only the user can enable it, in the Plugin Store (Installed tab). Tell the user; do not try to enable it yourself or work around it: respect their choice.";

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const code = (error: unknown) => (error as PluginError)?.code;

/** The player refused the id as one of its built-in plugins' (`reserved` vestra.*, or `unsupported`). */
const builtinId = (id: string): ToolResult => ({
  ok: false,
  text: `${id} has the name of a plugin built into the player and cannot be installed; choose another id.`,
  say: { code: "builtin-id", args: [id] },
});

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

/** The plugin's state after an install (`state` null: the list is unavailable). */
type Settled = {
  state: PluginInfo["state"] | "missing" | null;
  error: string | null;
  shadowed: Shadowed[];
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
      if (kind === "reserved") return { failure: builtinId(id) };
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
   * Where `id` stands once the install committed, as the player lists it:
   * its state once it has left "starting" (at most 5 s), and the inputs of
   * its commands another command holds (the player ignores them silently,
   * its log only; [] unless it runs). An update or reinstall of a plugin the
   * user disabled keeps it disabled and does not start it (player after
   * 0.6.3, as in VS Code); older players enable it on commit, so either
   * way the list says what is true. `state` is null when the list is
   * unavailable.
   */
  private async settle(id: string): Promise<Settled> {
    let entry: PluginInfo | undefined;
    try {
      for (let wait = 0; wait < 25; wait++) {
        entry = (await this.plugin.packages.list()).find((p) => p.id === id);
        if (entry?.state !== "starting") break;
        await new Promise((resolve) => setTimeout(resolve, 200));
      }
    } catch {
      return { state: null, error: null, shadowed: [] };
    }
    const state = entry?.state ?? "missing";
    const error = entry?.error ?? null;
    if (state !== "running") return { state, error, shadowed: [] };
    try {
      const { commands } = await this.plugin.bindings.list();
      const holder = (ref: { plugin: string; command: string }) =>
        commands.find((c) => c.plugin === ref.plugin && c.command === ref.command)?.title || ref.command;
      const shadowed = commands
        .filter((c) => c.plugin === id)
        .flatMap((c) => c.shadowed.map((s) => ({ input: s.input, title: c.title, plugin: s.by.plugin, holder: holder(s.by) })));
      return { state, error, shadowed };
    } catch {
      return { state, error, shadowed: [] };
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
        // `reserved`: a vestra.* id (the player refuses it already when
        // validating, see candidate()); `unsupported`: a built-in's id.
        if (code(error) === "reserved" || code(error) === "unsupported") {
          const failure = builtinId(id);
          this.say(failure.say!.code, failure.say!.args);
          return failure;
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
      const { state, error, shadowed } = await this.settle(id);
      if (state === "disabled") {
        // The user turned it off and an update keeps their choice; only
        // they turn it back on (the workshop never calls setEnabled).
        this.say("installed-disabled", [name, version]);
        return { ok, text: `${text}. ${DISABLED_TEXT}` };
      }
      if (state === "failed") {
        // Installed (ok: recorded for the delete dialog), but it stopped
        // with an error once it ran.
        this.say("installed-failed", [name, version, error ?? ""]);
        return {
          ok,
          text: `${text}, but the plugin then failed in the player${error ? `: ${error}` : ""}. Fix the cause in the source, run check and the tests again, then install again.`,
        };
      }
      this.say("installed", [name, version], shadowed);
      const keys = shadowed.map((s) => `${s.input} (${s.title}) is taken by ${s.plugin === "host" ? "the player" : s.plugin} "${s.holder}"`);
      return {
        ok,
        text:
          `${text}. ${(state && STATE_TEXT[state]) ?? "The player did not report the plugin's state."}` +
          (keys.length
            ? `\nShortcuts not in effect: ${keys.join("; ")}. The player ignored these keys (on a conflict the user's settings come first, then the player and the plugins loaded earlier). Tell the user and suggest keys that are free, then install again.`
            : ""),
      };
    });
  }
}
