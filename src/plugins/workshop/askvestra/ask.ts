// Ask Vestra (PROPOSAL §1–§4; §9 steps 4 and 5): the command palette's lead
// row hands the typed sentence to this plugin, a model of the user's choice
// turns it into steps (worker profile "ask") over the palette's command
// table (commands.ts), the steps run as the player's commands through
// `commands.run` with the request's ticket. What happens shows in the
// palette's status, which the player draws as a status strip where the
// palette was while the palette is closed (UX PROPOSAL §4.2): Planning…
// while the model plans, the player's own line for each step, then one end
// state (done, failed or a note) that the player times and removes itself.
// User ruling 2026-11-11: Ask Vestra runs commands only (no parameters, no
// direct playback control), so nothing it does is undone by it: there is
// no Undo.
//
// One request at a time: a new one stops the one running (PROPOSAL Q6 A).
import type { CommandInfo, Plugin, PaletteLeadOptions, PlayerState, Text, TextArg } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { AgentWorker } from "../agent.ts";
import type { StartMessage } from "../worker/protocol.ts";
import { MAX_STEPS, PlanError, parsePlan, type Step } from "./plan.ts";
import { askMessage, askSystemPrompt } from "./prompt.ts";
import { commandTable, offered } from "./commands.ts";

/** Wall time of one request (PROPOSAL §4). */
export const REQUEST_TIMEOUT_MS = 90_000;
/** Model calls per request: the plan and, after a malformed tool call, pi's retries of it (§4). */
export const MAX_MODEL_CALLS = 4;
/**
 * What the consent covers: bump it when a request sends more, so the user is
 * asked again (sending less needs no new consent). v1: the request and the
 * playback state; v2: also the command table.
 */
const CONSENT_SCOPE = "v2";
/**
 * The player's rate for one ticket: at most this many `commands.run` a
 * second (host `capabilities/commands_run.rs` RUNS_PER_SECOND; the SDK's
 * `commands.run` reference: `limit`, 4 a second). A plan has up to
 * MAX_STEPS (8), so its steps are paced to stay within it (`pace`).
 */
export const RUNS_PER_SECOND = 4;
/** Model text quoted in the status strip (an error, a refusal), at most. */
const QUOTE_CHARS = 120;

export const STOP_COMMAND = "askvestra.stop";
/**
 * A command in no menu, palette or bar (panel-only in a plugin without a panel, no
 * keys) whose only job is to be busy while a request runs: the supervisor
 * does not put a plugin in standby while one of its commands is busy
 * (doc/architecture/supervisor.md §4, declared work). Stop cannot carry the
 * flag: the player does not run a busy command.
 */
export const WORKING_COMMAND = "askvestra.working";

/** What Ask Vestra needs from the plugin's model settings (main.ts). */
export interface AskModel {
  /** A provider, model and key (or a keyless custom endpoint) are set up, by the settings alone: no keychain read. */
  setUp(): boolean;
  /** Reads the key if needed (a user action may wait on the keychain) and says whether a request can be sent. */
  ready(): Promise<boolean>;
  /** The connection for one request: provider, model, endpoint and key. */
  connection(): Promise<Pick<StartMessage, "provider" | "modelId" | "baseUrl" | "api" | "key" | "reasoning">>;
  /** Who receives the requests: the provider's id, and a custom endpoint's Base URL (consent is given per recipient). */
  consentKey(): string;
  /** The provider as the user knows it, for messages. */
  providerName(): TextArg;
  openSettings(): unknown;
  openWindow(): unknown;
}

interface Request {
  ticket: string;
  controller: AbortController;
  worker?: AgentWorker;
  /** Shown in the lead's status row: planning (null) or the step running. */
  step: { index: number; of: number; label: TextArg } | null;
  /** Stopped by the user (Stop), not replaced by a newer request. */
  stopped: boolean;
}

/** How a request ended, as the status strip shows it (`status.state`, SDK 1.7.15). */
export type EndState = "done" | "failed" | "note";
interface End {
  text: Text | string;
  state: EndState;
}

/** A failure the status strip names: the whole line; `failed` for a step that failed, else a `note`. */
class Failure extends Error {
  readonly text: Text | string;
  readonly state: EndState;
  constructor(text: Text | string, state: EndState = "note") {
    super("Ask Vestra failure");
    this.text = text;
    this.state = state;
  }
}

/** The user chose Don't Run in the player's confirmation: the request stops there. */
class Declined extends Error {
  readonly label: string;
  constructor(label: string) {
    super("Ask Vestra step declined");
    this.label = label;
  }
}

/** Why a step failed: a text, or the player's message as data. */
class Refusal extends Error {
  readonly reason: TextArg;
  constructor(reason: TextArg) {
    super("Ask Vestra step refused");
    this.reason = reason;
  }
}

const cut = (text: string) => {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > QUOTE_CHARS ? `${line.slice(0, QUOTE_CHARS - 1)}…` : line;
};
const codeOf = (error: unknown) => (error as { code?: string } | null)?.code;

/** Waits `ms`, or rejects with the signal's reason when it aborts first. */
const pause = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    signal.throwIfAborted();
    const aborted = () => {
      clearTimeout(timer);
      reject(signal.reason as Error);
    };
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", aborted);
      resolve();
    }, ms);
    signal.addEventListener("abort", aborted, { once: true });
  });

/**
 * Waits until one more `commands.run` stays within RUNS_PER_SECOND.
 * `answered`: when the player answered each earlier run of the ticket
 * (`performance.now()`). The player counts a run before it answers it and
 * the next one after it was sent, so a run sent one second after the answer
 * to the run RUNS_PER_SECOND before it is within the rate whatever the
 * delays between the processes. The first RUNS_PER_SECOND runs go at once.
 * A timer may fire a little early (Node measured 49.66 ms for 50), so the
 * time is read again after each pause and the wait goes on until it is
 * really over.
 */
export const pace = async (answered: readonly number[], signal: AbortSignal): Promise<void> => {
  signal.throwIfAborted();
  if (answered.length < RUNS_PER_SECOND) return;
  const until = answered[answered.length - RUNS_PER_SECOND]! + 1000;
  for (let wait = until - performance.now(); wait > 0; wait = until - performance.now()) {
    await pause(Math.ceil(wait), signal);
  }
};

export function askVestra(plugin: Plugin, model: AskModel) {
  const { i18n } = plugin;
  const consent = plugin.storage.json<{ providers: string[] }>("askvestra", {
    defaults: { providers: [] },
    validate: (v: unknown): v is { providers: string[] } => !!v && Array.isArray((v as { providers?: unknown }).providers),
  });
  let current: Request | null = null;
  /**
   * The latest request while it waits for the key or the consent (its
   * order, `latest`): the row already says Vestra is working, so a second
   * Enter on it stops instead of sending the request again.
   */
  let waiting: number | null = null;
  const working = () => current !== null || waiting !== null;

  // ---- the palette's lead row
  const leadOptions = (): PaletteLeadOptions => ({
    title: i18n.text("Ask Vestra"),
    icon: "ai",
    placeholder: i18n.text("Type a command or ask Vestra"),
    empty: i18n.text("Type a request or a question"),
    ...(model.setUp() ? {} : { note: i18n.text("Set up a model first") }),
    status: working()
      ? {
          text: current?.step
            ? i18n.text("Vestra is working · Step {0} of {1} · {2}", [current.step.index, current.step.of, current.step.label])
            : // Also the strip's text while the model plans; from the first step on the player writes the strip.
              i18n.text("Planning…"),
          note: i18n.text("Enter stops"),
        }
      : null,
  });
  let shownLead = "";
  const lead = plugin.palette.lead(leadOptions());
  shownLead = JSON.stringify(leadOptions());
  let leadHeld = true;
  // A development load is not built in: the player refuses the row (PROPOSAL Q10 A).
  lead.ready.catch((error) => {
    leadHeld = false;
    console.error(`Ask Vestra: the palette's lead row was refused: ${(error as Error).message}`);
  });
  /** Shows what changed (model set up or not, the running state). */
  const refresh = () => {
    if (!leadHeld) return;
    const options = leadOptions();
    const json = JSON.stringify(options);
    if (json === shownLead) return;
    shownLead = json;
    lead.update(options).catch((error) => console.error(`Ask Vestra: lead row: ${(error as Error).message}`));
    plugin.update();
  };

  /**
   * Shows how a request ended: once, as an end state of the row's status
   * (the player shows it in the strip, times it, and never keeps it on the
   * row), then the row as it is now (no status, or a newer request's).
   * Called once the request is no longer `current`, so the row's next form
   * is not this request's working status. No timer: `lead.done` may follow
   * at once.
   */
  const finish = async (end: End) => {
    if (!leadHeld) return;
    const options: PaletteLeadOptions = { ...leadOptions(), status: { text: end.text, state: end.state } };
    shownLead = JSON.stringify(options);
    await lead.update(options).catch((error) => console.error(`Ask Vestra: lead row: ${(error as Error).message}`));
    refresh();
  };

  // ---- one request
  /** A step as the strip names it:the command's label as the palette shows it (data). */
  const stepLabel = (step: Step, rows: ReadonlyMap<string, CommandInfo>): string => rows.get(step.id)?.label ?? step.id;

  /** The player state for the model, for toggles such as Play/Pause: no source name (§8). */
  const playerFacts = (state: PlayerState) => ({
    open: state.phase === "ready" || state.phase === "ended" || state.phase === "loading",
    playing: (state.phase === "ready" && !state.paused) || false,
  });

  /** The palette's table now, without personal rows; empty where the player refuses the list. */
  const table = async (): Promise<CommandInfo[]> => {
    try {
      return offered(await plugin.commands.list(), plugin.id);
    } catch (error) {
      console.error(`Ask Vestra: commands.list: ${(error as Error).message}`);
      return [];
    }
  };

  /**
   * Asks the model; resolves with its steps (validated against the table it
   * was given) and that table by id, rejects with a Failure or an abort.
   */
  const plan = async (request: Request, query: string): Promise<{ steps: Step[]; rows: Map<string, CommandInfo> }> => {
    const connection = await model.connection();
    const state = await plugin.player.refresh();
    const listed = await table();
    const rows = new Map(listed.map((row) => [row.id, row]));
    const directory = path.join(plugin.dataDirectory, "askvestra");
    await fs.mkdir(directory, { recursive: true });
    request.controller.signal.throwIfAborted();
    return new Promise((resolve, reject) => {
      let calls = 0;
      let lastText = "";
      let lastError: string | null = null;
      let settled = false;
      const end = (finish: () => void) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        request.controller.signal.removeEventListener("abort", aborted);
        finish();
      };
      const fail = (failure: Error) => end(() => reject(failure));
      const aborted = () => fail(request.controller.signal.reason as Error);
      const timer = setTimeout(() => fail(new Failure(i18n.text("Took too long"))), REQUEST_TIMEOUT_MS);
      request.controller.signal.addEventListener("abort", aborted, { once: true });
      const reachFailure = (message: string) => new Failure(i18n.text("Could not reach {0}: {1}", [model.providerName(), cut(message)]));
      const worker = new AgentWorker(
        {
          ...connection,
          profile: "ask",
          sessionDir: directory,
          workDir: directory,
          sessionFile: null,
          // A one-line request: a short think at most (a model without reasoning runs without).
          thinking: "low",
          systemPrompt: askSystemPrompt(commandTable(listed)),
          docsRoots: [],
          pluginTools: ["run_steps"],
        },
        {
          tool: (call, name, args) => {
            worker.reply(call, { ok: true, text: "Running the steps." });
            if (name !== "run_steps") return fail(noSteps());
            try {
              const steps = parsePlan(args, new Set(rows.keys()));
              end(() => resolve({ steps, rows }));
            } catch (error) {
              fail(error as Error);
            }
          },
          event: (event) => {
            if (event.kind === "assistant_start" && ++calls > MAX_MODEL_CALLS) {
              worker.abort();
              fail(noSteps());
            } else if (event.kind === "assistant_end") {
              lastText = event.text;
              lastError = event.error;
            } else if (event.kind === "settled") {
              if (lastError) fail(reachFailure(lastError));
              else if (lastText.trim()) fail(new Failure(cut(lastText))); // the model's sentence as it wrote it
              else fail(noSteps());
            }
          },
          file: () => {},
          failed: (message) => fail(reachFailure(message)),
          exited: (code) => fail(reachFailure(worker.lastError() || `exit ${code}`)),
        },
      );
      request.worker = worker;
      worker.prompt(askMessage(query, playerFacts(state))).catch(() => {}); // start failures arrive through `failed`
    });
  };

  /**
   * Runs a command step as the palette would, with the request's ticket:
   * the player checks it (enabled, not busy) and asks the user first for a
   * dangerous one (Don't Run: Declined). `index`/`of` go to the player's
   * own line in the status strip for a request of several steps.
   */
  const runCommand = async (id: string, label: string, ticket: string, index: number, of: number, signal: AbortSignal): Promise<void> => {
    signal.throwIfAborted();
    let result;
    try {
      result = await plugin.commands.run(id, of > 1 ? { ticket, step: index, of } : { ticket });
    } catch (error) {
      const code = codeOf(error);
      if (code === "disabled") throw new Refusal(cut((error as Error).message)); // the command's reason, as shown
      if (code === "busy") throw new Refusal(i18n.text("{} is busy", [label]));
      if (code === "missing") throw new Refusal(i18n.text("{} is no longer there", [label]));
      if (code === "limit") throw new Refusal(i18n.text("Too many commands at once"));
      if (code === "ticket") throw new Refusal(i18n.text("The request has ended"));
      if (code === "capability") throw new Refusal(i18n.text("This player does not let Vestra run commands"));
      throw new Refusal(cut((error as Error).message));
    }
    if (!result.ran) throw new Declined(label);
  };

  const stopWorker = (request: Request) => {
    const worker = request.worker;
    request.worker = undefined;
    worker?.stop(300).catch(() => {});
  };

  /** Stops the running request: `user` from Stop (the strip says so), else a newer request replaces it. */
  const stop = (user: boolean) => {
    const request = current;
    if (!request) return;
    request.stopped = user;
    current = null;
    request.controller.abort(new DOMException("Stopped", "AbortError"));
    stopWorker(request);
    refresh();
  };

  /** Stop from the user: the running request, and one waiting for the key or the consent. */
  const stopByUser = () => {
    if (waiting !== null) {
      waiting = null;
      latest++; // the waiting request goes no further
      refresh();
      // The strip said Planning… for it; a running request says Stopped itself.
      if (!current) void finish({ text: i18n.text("Stopped"), state: "note" });
    }
    stop(true);
  };

  /**
   * Asks once per recipient before the first request goes to it (PROPOSAL
   * §8, Q12 A). Requests made while the dialog is open share its answer.
   */
  let asking: { key: string; answer: Promise<boolean> } | null = null;
  const consented = (): Promise<boolean> => {
    const key = `${CONSENT_SCOPE} ${model.consentKey()}`;
    if (consent.value.providers.includes(key)) return Promise.resolve(true);
    if (asking?.key === key) return asking.answer;
    const answer = (async () => {
      try {
        const { confirmed } = await plugin.dialogs.confirm({
          message: i18n.text("Ask Vestra sends your request to {}", [model.providerName()]),
          detail: i18n.text(
            "What you type in the command palette, the player's command list (names, menus, keys and whether each is on or available) and whether something is open and playing go to this provider with your API key, as in the chat. File names, playlists, history, track and chapter titles and subtitles are not sent. The provider charges your account.",
          ),
          confirm: i18n.text("Continue"),
        });
        if (confirmed && !consent.value.providers.includes(key)) consent.save({ providers: [...consent.value.providers, key] });
        return confirmed;
      } finally {
        if (asking?.key === key) asking = null;
      }
    })();
    asking = { key, answer };
    return answer;
  };

  /** Requests in the order the user made them: only the latest goes on after a wait (keychain, consent). */
  let latest = 0;
  const request = async (query: string, ticket: string) => {
    const order = ++latest;
    waiting = order;
    refresh();
    /** This request waits no longer (a newer one may wait now: its own). */
    const waited = () => {
      if (waiting !== order) return;
      waiting = null;
      refresh();
    };
    // A user action: reading the key may wait on the keychain's dialog.
    const ready = await model.ready().catch(() => false);
    if (order !== latest) return;
    if (!ready) {
      waited();
      // Flow 7: the settings page (the palette keeps the typed text, Q8);
      // set up but the key cannot be read: say so.
      if (!model.setUp()) await model.openSettings();
      else await finish({ text: i18n.text("Cannot read the API key; save it again in the settings"), state: "note" });
      return;
    }
    let consentGiven: boolean;
    try {
      consentGiven = await consented();
    } finally {
      // Also when the dialog is refused (`busy`: another native dialog is
      // open): the row and the busy command must not stay "working".
      waited();
    }
    if (!consentGiven || order !== latest) return;
    // This request replaces the one running, if any (PROPOSAL Q6 A).
    stop(false);
    const mine: Request = { ticket, controller: new AbortController(), step: null, stopped: false };
    current = mine;
    refresh();
    const labels: string[] = [];
    let end: End | null = null;
    try {
      const { steps, rows } = await plan(mine, query);
      stopWorker(mine);
      let opened: string | null = null;
      /** When the player answered each step's run, for the ticket's rate. */
      const answered: number[] = [];
      for (const [index, step] of steps.entries()) {
        mine.controller.signal.throwIfAborted();
        const label = stepLabel(step, rows);
        await pace(answered, mine.controller.signal);
        mine.step = { index: index + 1, of: steps.length, label };
        refresh();
        try {
          // The player shows its own line for each command it runs (step/of) in the strip.
          await runCommand(step.id, label, ticket, index + 1, steps.length, mine.controller.signal);
        } catch (error) {
          if (error instanceof Refusal) throw new Failure(i18n.text("Step {0} failed: {1}", [index + 1, error.reason]), "failed");
          throw error;
        }
        answered.push(performance.now());
        labels.push(label);
        // A command that opens a window ends the request: the user is in it now (PROPOSAL §4).
        if (rows.get(step.id)?.title.endsWith("…")) {
          opened = label;
          break;
        }
      }
      mine.controller.signal.throwIfAborted(); // stopped during the last step: "Stopped", not the result
      end =
        opened !== null
          ? { text: i18n.text("Opened {}", [opened]), state: "done" }
          : { text: i18n.text("Done · {}", [i18n.list(labels)]), state: "done" };
    } catch (error) {
      if (mine.controller.signal.aborted) {
        // Replaced by a newer request: nothing to say, the newer one shows.
        if (mine.stopped) end = { text: i18n.text("Stopped"), state: "note" };
      } else if (error instanceof Declined) {
        // Don't Run in the player's confirmation, asked before that step:
        // "nothing was run" only while it is true.
        end = labels.length
          ? { text: i18n.text("Stopped: {} was not run", [error.label]), state: "note" }
          : { text: i18n.text("Stopped: nothing was run"), state: "note" };
      } else if (error instanceof Failure) end = { text: error.text, state: error.state };
      else if (error instanceof PlanError) end = planFailure(error);
      else {
        console.error(`Ask Vestra: ${(error as Error).stack ?? error}`);
        end = { text: cut((error as Error).message), state: "note" };
      }
    } finally {
      stopWorker(mine);
      if (current === mine) current = null;
    }
    // The row stops saying "working" with the end state (at once without one).
    if (end) await finish(end);
    else refresh();
  };

  const noSteps = () => new Failure(i18n.text("The model gave no steps"));
  const planFailure = (error: PlanError): End => {
    switch (error.code) {
      case "too-many":
        return { text: i18n.text("The plan has more than {} steps", [MAX_STEPS]), state: "note" };
      case "unknown-kind":
        return { text: i18n.text("Step {0} failed: {1}", [error.step, i18n.text("Vestra cannot do “{}”", [error.value])]), state: "failed" };
      case "unknown-command":
        return { text: i18n.text("Step {0} failed: {1}", [error.step, i18n.text("There is no command “{}”", [error.value])]), state: "failed" };
      default:
        return { text: noSteps().text, state: "note" };
    }
  };

  lead.onRun(({ query, ticket, status }) => {
    const text = query.trim();
    (async () => {
      try {
        if (status) stopByUser(); // Enter on the status row (flow 3)
        else if (!text) await model.openWindow(); // Q2 A: the empty row opens the AI window
        else await request(text, ticket);
      } finally {
        await lead.done(ticket).catch(() => {});
      }
    })().catch((error) => console.error(`Ask Vestra: ${(error as Error).stack ?? error}`));
  });

  // ---- Stop: a palette command (the strip's Stop is Enter on the status row)
  plugin.commands.add({
    id: STOP_COMMAND,
    title: "Stop",
    // i18n-ignore: the assistant's name, the same in every language
    category: "Vestra",
    enabled: working,
    reason: "Vestra is not working on a request",
    run: stopByUser,
  });
  // Busy while a request runs: the player does not put the plugin in standby meanwhile (WORKING_COMMAND).
  plugin.commands.add({
    id: WORKING_COMMAND,
    title: "Vestra is working",
    // i18n-ignore: the assistant's name, the same in every language
    category: "Vestra",
    focus: "panel", // in no menu, palette or bar: the plugin has no panel
    busy: working,
    reason: "Vestra is working",
    run: () => {},
  });

  plugin.onDispose(() => {
    if (current) stop(false);
  });

  return {
    /** The model settings changed: the row's note follows. */
    refresh,
  };
}
