// The texts the windows show for the plugin's codes, as a user of the test
// host's language reads them: the templates are read from the Slint files'
// code → @tr chains (ai.slint, settings.slint), so the tests follow the real
// mapping; the test host translates them with the plugin's catalog.
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import { TestHost } from "@vestra-player/sdk/testing";

const read = (file) => fs.readFileSync(new URL(`../${file}`, import.meta.url), "utf8");
const unquote = (text) => JSON.parse(`"${text}"`);

/** code → English template of `pure function <fn>` in `file` (simple `x.code == "…" ? @tr("…", …)` branches). */
export function templates(file, fn) {
  const source = read(file);
  const start = source.indexOf(`pure function ${fn}(`);
  if (start < 0) throw new Error(`${file}: no function ${fn}`);
  const next = source.indexOf("pure function", start + 1);
  const body = source.slice(start, next < 0 ? undefined : next);
  const map = new Map();
  for (const match of body.matchAll(/code == "([^"]+)" \? @tr\("((?:[^"\\]|\\.)*)"/g)) if (!map.has(match[1])) map.set(match[1], unquote(match[2]));
  return map;
}

/** The text of `say` ({ code, args }) as `fn` of `file` shows it, in the host's language. */
export function shown(host, file, fn, say) {
  const template = templates(file, fn).get(say.code);
  if (template === undefined) throw new Error(`${file} ${fn}: no template for ${say.code}`);
  return host.translate(template, say.args);
}

export const banner = (host, say) => shown(host, "ai.slint", "banner-text", say);
/** The result card's line, as AiWindow.card-text in ai.slint (installs and rollbacks in two parts). */
export function card(host, say, keys = []) {
  if (say.code === "installed" && keys.length)
    return host.translate("Installed {0} {1}; you can click Roll Back (Undo Install). Shortcuts not in effect:", say.args);
  if (say.code !== "result" && say.code !== "progress") return shown(host, "ai.slint", "card-text", say);
  const [action, a, b, status, detail] = say.args;
  const what =
    action === "install" ? host.translate("Install {0} {1}", [a, b]) : action === "rollback" ? host.translate("Roll back to {}", [a]) : host.translate("Uninstall {}", [a]);
  if (say.code === "progress") return host.translate("{0}: {1}…", [what, status]);
  const outcomes = { committed: "done", cancelled: "cancelled", failed: "failed", failedDegraded: "failed; the previous version could not be fully restored", tooLate: "too late to cancel" };
  const outcome = outcomes[status] ? host.translate(outcomes[status]) : status;
  return detail ? host.translate("{0}: {1} ({2})", [what, outcome, detail]) : host.translate("{0}: {1}", [what, outcome]);
}
export const notice = (host, row) => (row.code === "hidden" ? hidden(host, row.n) : shown(host, "ai.slint", "notice-text", row));
export const error = (host, row) => (row.code === "failed" ? host.translate("Something went wrong.") : shown(host, "ai.slint", "error-text", row));
export const keyState = (host, say) => shown(host, "settings.slint", "key-text", say);
/** A provider's name as Labels.provider (labels.slint) gives it. */
export const providerName = (host, id, name) =>
  id === "custom-openai" ? host.translate("Custom · OpenAI format") : id === "custom-anthropic" ? host.translate("Custom · Anthropic format") : name;
export const message = (host, say) =>
  ["key-saved", "key-deleted"].includes(say.code)
    ? shown(host, "settings.slint", "message-text", { ...say, args: [providerName(host, say.args[0], say.args[1])] })
    : say.code === "fetched"
      ? found(host, say.n)
      : say.code === "fetched-replaced"
        ? host.translate("{0} “{1}” is not in the list; now using “{2}”.", [found(host, say.n), ...say.args])
        : shown(host, "settings.slint", "message-text", say);
const found = (host, n) => plural(host, "Found {n} model.", "Found {n} models.", n);
export const ownerHint = (host, say) => shown(host, "settings.slint", "owner-text", say);

/** A plural text ("{n} …"): the catalog's form, else the English one for `n`. */
function plural(host, one, other, n) {
  const translated = host.translate(one);
  const template = translated === one && n !== 1 ? other : translated;
  return host.translate(template, { n });
}

/** A tool call's label, as Message.label in ai.slint: "Read main.ts". */
export function callLabel(host, call) {
  const source = read("ai.slint");
  const verbs = new Map([...source.matchAll(/tool == "([^"]+)" \? @tr\("([^"]+)"\)/g)].map((m) => [m[1], m[2]]));
  const verb = verbs.has(call.tool) ? host.translate(verbs.get(call.tool)) : call.tool;
  if (!call.target) return verb;
  return host.translate("{0} {1}", [verb, call.reference ? host.translate("reference/{}", [call.target]) : call.target]);
}

const mark = (state) => (state === 0 ? "…" : state === 1 ? "✓" : "✗");

/** The tool line of a "tools" row, as Message.tools-text in ai.slint. */
export function toolLine(host, row) {
  const phase = host.translate(row.phase === "replying" ? "Replying…" : "Thinking…");
  if (row.code === "running") return `${callLabel(host, row.call)}…`;
  if (row.code === "after") return `${callLabel(host, row.call)} ${row.call.state === 2 ? "✗" : "✓"} · ${phase}`;
  if (row.code === "phase") return phase;
  const calls = plural(host, "{n} tool call", "{n} tool calls", row.n);
  return row.failed > 0 ? host.translate("{0}, {1} failed", [calls, row.failed]) : calls;
}

/** The expanded list of a "tools" row: one line per call. */
export const toolList = (host, row) => row.calls.map((call) => `${mark(call.state)} ${callLabel(host, call)}`).join("\n");

/** The "hidden" notice. */
export const hidden = (host, n) =>
  plural(host, "{n} earlier message is not shown (the full record is in the session folder)", "{n} earlier messages are not shown (the full record is in the session folder)", n);

/** A test host that only translates: the plugin's catalog for `locale`. */
export function translator(locale = "zh-Hans") {
  const host = new TestHost();
  host.locale = locale;
  host.loadCatalog(fileURLToPath(new URL("..", import.meta.url)));
  return host;
}

/** A binding that did not take effect, as AiWindow.shadowed-text in ai.slint. */
export const shadowedText = (host, k) =>
  host.translate("{0} ({1}) is taken by {2}", [k.input, k.title, k.plugin === "host" ? host.translate("the player “{}”", [k.holder]) : host.translate("{0} “{1}”", [k.plugin, k.holder])]);
