// What the player would do with Ask Vestra's status texts, checked after each
// test of a file that calls `guard(test)`: every `palette.lead.set` status
// text, translated and filled in, is 1–80 characters counted as the player
// counts them (Unicode scalar values: host capabilities/palette_lead.rs
// `text`), and the plugin logged no refused lead row ("lead row: …", the
// SDK's `update` rejected). The TestHost does not measure the shown text the
// player's way, so a plugin call it accepts may still be refused by the
// player; this checks the player's rule.
import assert from "node:assert/strict";
import { STATUS_CHARS } from "../askvestra/status.ts";

/** A text of the lead row (wire form: a string or { $vestraText, args }) as the palette shows it. */
export const shown = (host, value) => {
  if (value === undefined || value === null) return value;
  return typeof value === "string" ? host.translate(value) : host.translate(value.$vestraText, value.args ?? []);
};

const hosts = [];
const refused = [];

/** Checks the hosts `track` was given after each test, then forgets them. */
export function guard(test) {
  const original = console.error;
  console.error = (...args) => {
    const line = args.map(String).join(" ");
    if (line.includes("lead row: ")) refused.push(line);
    original(...args);
  };
  test.afterEach(() => {
    const seen = hosts.splice(0);
    const errors = refused.splice(0);
    assert.deepEqual(errors, [], "the player refused a lead row");
    for (const host of seen)
      for (const params of host.sent("palette.lead.set")) {
        if (!params.status) continue;
        const text = shown(host, params.status.text);
        const length = [...text].length;
        assert.ok(length >= 1 && length <= STATUS_CHARS && !/[\0\n\r]/.test(text), `status.text the player refuses (${length} characters): ${text}`);
      }
  });
}

/** A host whose lead statuses `guard` checks after the test. */
export const track = (host) => {
  hosts.push(host);
  return host;
};
