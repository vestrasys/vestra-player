// Ask Vestra's plan: the steps the model asked for (worker/plugin-tools.ts
// ASK_TOOLS.run_steps), checked before any of them runs (PROPOSAL §4).
// Two kinds of step: `speed` (the plugin's own player.setRate) and
// `command` (a command of the table the model was given, run through
// plugin.commands.run).

/** The most steps one request may run; a longer plan is refused. */
export const MAX_STEPS = 8;
/** The speeds Ask Vestra sets: the player's own keys step within this range. */
export const RATE = { min: 0.25, max: 4 } as const;

export type Step = { kind: "speed"; rate: number } | { kind: "command"; id: string };

/**
 * Why a plan is refused, for the OSD: `step` counts from 1 (0: the whole
 * plan), `value` is the offending value as data.
 */
export class PlanError extends Error {
  readonly code: "no-steps" | "too-many" | "unknown-kind" | "rate-range" | "unknown-command";
  readonly step: number;
  readonly value: string;
  constructor(code: PlanError["code"], step = 0, value = "") {
    super(`${code}${step ? ` (step ${step})` : ""}${value ? `: ${value}` : ""}`);
    this.code = code;
    this.step = step;
    this.value = value;
  }
}

/** Two decimals at most: 1.25×, not 1.2500001×. */
export const roundRate = (rate: number) => Math.round(rate * 100) / 100;

/**
 * The steps of `args` ({ steps: [...] }), or a PlanError for the first
 * problem. `commands`: the ids of the table the model was given; a command
 * step naming any other id is refused before any step runs.
 */
export function parsePlan(args: unknown, commands: ReadonlySet<string>): Step[] {
  const raw = (args as { steps?: unknown } | null)?.steps;
  if (!Array.isArray(raw) || raw.length === 0) throw new PlanError("no-steps");
  if (raw.length > MAX_STEPS) throw new PlanError("too-many", 0, String(raw.length));
  return raw.map((item, index): Step => {
    const step = (item ?? {}) as Record<string, unknown>;
    const kind = typeof step.kind === "string" ? step.kind : "";
    if (kind === "command") {
      const id = typeof step.id === "string" ? step.id : "";
      if (!commands.has(id)) throw new PlanError("unknown-command", index + 1, id.slice(0, 80));
      return { kind, id };
    }
    if (kind !== "speed") throw new PlanError("unknown-kind", index + 1, kind.slice(0, 40));
    const rate = typeof step.rate === "number" ? step.rate : Number.NaN;
    if (!Number.isFinite(rate) || rate < RATE.min || rate > RATE.max) throw new PlanError("rate-range", index + 1, String(step.rate).slice(0, 20));
    return { kind, rate: roundRate(rate) };
  });
}
