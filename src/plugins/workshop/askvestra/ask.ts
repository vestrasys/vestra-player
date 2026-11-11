// Ask Vestra (PROPOSAL §1–§4; this file is §9 step 4, the vertical slice):
// the command palette's lead row hands the typed sentence to this plugin, a
// model of the user's choice turns it into steps (worker profile "ask"),
// the steps run with the plugin's own playback control, and the OSD names
// what ran with Undo. Commands (`commands.run`), the picture builder,
// presets and the chat record come in later steps.
//
// One request at a time: a new one stops the one running (PROPOSAL Q6 A).
// The undo journal lives in session storage, so it survives the plugin's
// idle standby but not a player restart (§7).
import type { Plugin, PaletteLeadOptions, PlayerState, Text, TextArg } from "@vestra-player/sdk";
import fs from "node:fs/promises";
import path from "node:path";
import { AgentWorker } from "../agent.ts";
import type { StartMessage } from "../worker/protocol.ts";
import { MAX_STEPS, PlanError, RATE, parsePlan, type Step } from "./plan.ts";
import { ASK_SYSTEM_PROMPT, askMessage } from "./prompt.ts";

/** Wall time of one request (PROPOSAL §4). */
export const REQUEST_TIMEOUT_MS = 90_000;
/** Model calls per request: the plan and, after a malformed tool call, pi's retries of it (§4). */
export const MAX_MODEL_CALLS = 4;
/** Requests kept for Undo (§7). */
export const JOURNAL_SIZE = 10;
/** Model text quoted on the OSD (an error, a refusal), at most. */
const QUOTE_CHARS = 120;

export const UNDO_COMMAND = "askvestra.undo";
export const STOP_COMMAND = "askvestra.stop";

/** What Ask Vestra needs from the plugin's model settings (main.ts). */
export interface AskModel {
  /** A provider, model and key (or a keyless custom endpoint) are set up, by the settings alone: no keychain read. */
  setUp(): boolean;
  /** Reads the key if needed (a user action may wait on the keychain) and says whether a request can be sent. */
  ready(): Promise<boolean>;
  /** The connection for one request: provider, model, endpoint and key. */
  connection(): Promise<Pick<StartMessage, "provider" | "modelId" | "baseUrl" | "api" | "key" | "reasoning">>;
  /** The provider's id (consent is given per provider). */
  providerId(): string;
  /** The provider as the user knows it, for messages. */
  providerName(): TextArg;
  openSettings(): unknown;
  openWindow(): unknown;
}

/** One undoable change: what to restore. Only the plugin's own controls in v1 (coordinator ruling). */
type JournalStep = { kind: "rate"; before: number };
type JournalEntry = { steps: JournalStep[] };

interface Request {
  ticket: string;
  controller: AbortController;
  worker?: AgentWorker;
  /** Shown in the lead's status row: planning (null) or the step running. */
  step: { index: number; of: number; label: Text } | null;
  /** Stopped by the user (Stop), not replaced by a newer request. */
  stopped: boolean;
}

/** A failure the OSD names: the whole line as the OSD says it. */
class Failure extends Error {
  readonly text: Text;
  constructor(text: Text) {
    super("Ask Vestra failure");
    this.text = text;
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

export function askVestra(plugin: Plugin, model: AskModel) {
  const { i18n } = plugin;
  const consent = plugin.storage.json<{ providers: string[] }>("askvestra", {
    defaults: { providers: [] },
    validate: (v: unknown): v is { providers: string[] } => !!v && Array.isArray((v as { providers?: unknown }).providers),
  });
  let current: Request | null = null;

  // ---- the palette's lead row
  const leadOptions = (): PaletteLeadOptions => ({
    title: i18n.text("Ask Vestra"),
    icon: "ai",
    placeholder: i18n.text("Type a command or ask Vestra"),
    empty: i18n.text("Type a request or a question"),
    ...(model.setUp() ? {} : { note: i18n.text("Set up a model first") }),
    status: current
      ? {
          text: current.step
            ? i18n.text("Vestra is working · Step {0} of {1} · {2}", [current.step.index, current.step.of, current.step.label])
            : i18n.text("Vestra is working · Planning…"),
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

  type Action = { title: Text; command: string };
  const osd = (text: Text, actions: Action[] = []) =>
    plugin.osd.show(text, null, actions.length ? { actions } : undefined).catch((error) => console.error(`Ask Vestra: OSD: ${(error as Error).message}`));
  const undoAction = (): Action => ({ title: i18n.text("Undo"), command: UNDO_COMMAND });
  const stopAction = (): Action => ({ title: i18n.text("Stop"), command: STOP_COMMAND });

  // ---- the undo journal (session storage: it survives standby, not a restart)
  const JOURNAL = "askvestra.journal";
  const loadJournal = async () => ((await plugin.storage.session.get<JournalEntry[]>(JOURNAL)) ?? []).filter((e) => Array.isArray(e?.steps));
  const record = async (entry: JournalEntry) => {
    const journal = await loadJournal();
    journal.push(entry);
    await plugin.storage.session.set(JOURNAL, journal.slice(-JOURNAL_SIZE));
  };

  // ---- one request
  const stepLabel = (step: Step): Text => i18n.text("Speed {}×", [step.rate]);

  /** The player state for the model: no source name (§8). */
  const playerFacts = (state: PlayerState) => ({
    open: state.phase === "ready" || state.phase === "ended" || state.phase === "loading",
    playing: (state.phase === "ready" && !state.paused) || false,
    position: state.position === null ? null : Math.round(state.position),
    duration: state.duration === null ? null : Math.round(state.duration),
    rate: state.rate,
  });

  /** Asks the model; resolves with its steps (validated), rejects with a Failure or an abort. */
  const plan = async (request: Request, query: string): Promise<Step[]> => {
    const connection = await model.connection();
    const state = await plugin.player.refresh();
    const directory = path.join(plugin.dataDirectory, "askvestra");
    await fs.mkdir(directory, { recursive: true });
    request.controller.signal.throwIfAborted();
    return new Promise<Step[]>((resolve, reject) => {
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
      const timer = setTimeout(() => fail(new Failure(i18n.text("Vestra · Took too long"))), REQUEST_TIMEOUT_MS);
      request.controller.signal.addEventListener("abort", aborted, { once: true });
      const reachFailure = (message: string) => new Failure(i18n.text("Vestra · Could not reach {0}: {1}", [model.providerName(), cut(message)]));
      const worker = new AgentWorker(
        {
          ...connection,
          profile: "ask",
          sessionDir: directory,
          workDir: directory,
          sessionFile: null,
          // A one-line request: a short think at most (a model without reasoning runs without).
          thinking: "low",
          systemPrompt: ASK_SYSTEM_PROMPT,
          docsRoots: [],
          pluginTools: ["run_steps"],
        },
        {
          tool: (call, name, args) => {
            worker.reply(call, { ok: true, text: "Running the steps." });
            if (name !== "run_steps") return fail(noSteps());
            try {
              const steps = parsePlan(args);
              end(() => resolve(steps));
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
              else if (lastText.trim()) fail(new Failure(i18n.text("Vestra · {}", [cut(lastText)])));
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

  /** Runs one step with the plugin's own control; returns what undoes it. */
  const runStep = async (step: Step): Promise<JournalStep> => {
    const state = await plugin.player.refresh();
    if (state.phase === "empty" || state.phase === "error") throw new Refusal(i18n.text("Nothing is playing"));
    if (!state.can.rate) throw new Refusal(i18n.text("This source cannot change speed"));
    const before = state.rate;
    try {
      await plugin.player.setRate(step.rate, { ifCurrent: state });
    } catch (error) {
      const code = codeOf(error);
      if (code === "stale") throw new Refusal(i18n.text("Playback changed meanwhile"));
      if (code === "unsupported") throw new Refusal(i18n.text("This source cannot change speed"));
      if (code === "invalid") throw new Refusal(i18n.text("Nothing is playing"));
      throw new Refusal(cut((error as Error).message));
    }
    return { kind: "rate", before };
  };

  const stopWorker = (request: Request) => {
    const worker = request.worker;
    request.worker = undefined;
    worker?.stop(300).catch(() => {});
  };

  /** Stops the running request: `user` from Stop (the OSD says so), else a newer request replaces it. */
  const stop = (user: boolean) => {
    const request = current;
    if (!request) return;
    request.stopped = user;
    current = null;
    request.controller.abort(new DOMException("Stopped", "AbortError"));
    stopWorker(request);
    refresh();
  };

  /** Asks once per provider before the first request goes to it (PROPOSAL §8, Q12 A). */
  const consented = async () => {
    const id = model.providerId();
    if (consent.value.providers.includes(id)) return true;
    const answer = await plugin.dialogs.confirm({
      message: i18n.text("Ask Vestra sends your request to {}", [model.providerName()]),
      detail: i18n.text(
        "What you type in the command palette, the player's command list and the playback state (speed, position, volume, tracks) go to this provider with your API key, as in the chat. File names, playlists, history and subtitles are not sent. The provider charges your account.",
      ),
      confirm: i18n.text("Continue"),
    });
    if (!answer.confirmed) return false;
    consent.save({ providers: [...consent.value.providers, id] });
    return true;
  };

  const request = async (query: string, ticket: string) => {
    // A user action: reading the key may wait on the keychain's dialog.
    if (!(await model.ready().catch(() => false))) {
      refresh();
      // Flow 7: the settings page (the palette keeps the typed text, Q8);
      // set up but the key cannot be read: say so.
      if (!model.setUp()) await model.openSettings();
      else await osd(i18n.text("Vestra · Cannot read the API key; save it again in the settings"));
      return;
    }
    if (!(await consented())) return;
    // A request that came in while the dialog was open replaced this one.
    stop(false);
    const mine: Request = { ticket, controller: new AbortController(), step: null, stopped: false };
    current = mine;
    refresh();
    const done: JournalStep[] = [];
    const labels: Text[] = [];
    try {
      await osd(i18n.text("Vestra · Planning…"), [stopAction()]);
      const steps = await plan(mine, query);
      stopWorker(mine);
      for (const [index, step] of steps.entries()) {
        mine.controller.signal.throwIfAborted();
        const label = stepLabel(step);
        mine.step = { index: index + 1, of: steps.length, label };
        refresh();
        if (steps.length > 1) await osd(i18n.text("Vestra · {0}/{1} · {2}", [index + 1, steps.length, label]), [stopAction()]);
        try {
          done.push(await runStep(step));
        } catch (error) {
          if (error instanceof Refusal) throw new Failure(i18n.text("Vestra · Step {0} failed: {1}", [index + 1, error.reason]));
          throw error;
        }
        labels.push(label);
      }
      if (done.length) await record({ steps: done });
      await osd(i18n.text("Vestra · {}", [i18n.list(labels)]), [undoAction()]);
    } catch (error) {
      if (done.length) await record({ steps: done });
      const actions = done.length ? [undoAction()] : [];
      if (mine.controller.signal.aborted) {
        if (mine.stopped) await osd(i18n.text("Vestra · Stopped"), actions);
      } else if (error instanceof Failure) await osd(error.text, actions);
      else if (error instanceof PlanError) await osd(planFailure(error), actions);
      else {
        console.error(`Ask Vestra: ${(error as Error).stack ?? error}`);
        await osd(i18n.text("Vestra · {}", [cut((error as Error).message)]), actions);
      }
    } finally {
      stopWorker(mine);
      if (current === mine) {
        current = null;
        refresh();
      }
    }
  };

  const noSteps = () => new Failure(i18n.text("Vestra · The model gave no steps"));
  const planFailure = (error: PlanError): Text => {
    switch (error.code) {
      case "too-many":
        return i18n.text("Vestra · The plan has more than {} steps", [MAX_STEPS]);
      case "unknown-kind":
        return i18n.text("Vestra · Step {0} failed: {1}", [error.step, i18n.text("Vestra cannot do “{}” yet", [error.value])]);
      case "rate-range":
        return i18n.text("Vestra · Step {0} failed: {1}", [error.step, i18n.text("The speed must be between {0}× and {1}×", [RATE.min, RATE.max])]);
      default:
        return noSteps().text;
    }
  };

  lead.onRun(({ query, ticket, status }) => {
    const text = query.trim();
    (async () => {
      try {
        if (status) stop(true); // Enter on the status row (flow 3)
        else if (!text) await model.openWindow(); // Q2 A: the empty row opens the AI window
        else await request(text, ticket);
      } finally {
        await lead.done(ticket).catch(() => {});
      }
    })().catch((error) => console.error(`Ask Vestra: ${(error as Error).stack ?? error}`));
  });

  // ---- Undo and Stop: palette commands, and the OSD's buttons
  const undoLast = async () => {
    const journal = await loadJournal();
    const entry = journal.pop();
    if (!entry) return osd(i18n.text("Vestra · Nothing to undo"));
    await plugin.storage.session.set(JOURNAL, journal);
    try {
      for (const step of [...entry.steps].reverse()) {
        const state = await plugin.player.refresh();
        if (state.phase === "empty" || state.phase === "error") throw new Refusal(i18n.text("Nothing is playing"));
        await plugin.player.setRate(step.before, { ifCurrent: state });
      }
      await osd(i18n.text("Vestra · Undone"));
    } catch (error) {
      await osd(i18n.text("Vestra · Undo failed: {}", [error instanceof Refusal ? error.reason : cut((error as Error).message)]));
    }
  };
  plugin.commands.add({
    id: UNDO_COMMAND,
    title: "Undo Last Request",
    // i18n-ignore: the assistant's name, the same in every language
    category: "Vestra",
    // Busy while a request runs: the player does not put the plugin in standby meanwhile.
    busy: () => !!current,
    reason: "Vestra is working",
    run: undoLast,
  });
  plugin.commands.add({
    id: STOP_COMMAND,
    title: "Stop",
    // i18n-ignore: the assistant's name, the same in every language
    category: "Vestra",
    enabled: () => !!current,
    reason: "Vestra is not working on a request",
    run: () => stop(true),
  });

  plugin.onDispose(() => {
    if (current) stop(false);
  });

  return {
    /** The model settings changed: the row's note follows. */
    refresh,
  };
}
