// Assistant messages as Markdown documents (R-a, SDK 1.5.4): one document per
// message, made again only when its text changed, no remote images, a stable
// row shape, and the player's limits for one value.
import assert from "node:assert/strict";
import test from "node:test";
import { Chat } from "../chat.ts";
import { MAX_DOC_BYTES, MAX_FIELD_BYTES, MAX_ROWS, MAX_TEXT_BYTES, MessageDocs } from "../messages.ts";
import { server, startAi } from "./fixture.mjs";

/** MessageDocs with a fake ui.markdown that counts its calls. */
function counted() {
  const made = [];
  const docs = new MessageDocs(async (text) => {
    made.push(text);
    return { text, __vestraMarkdown: true };
  });
  return { docs, made };
}

function conversation(turns, size = 10) {
  const chat = new Chat();
  for (let i = 0; i < turns; i++) {
    chat.user(`问题 ${i}`);
    chat.begin();
    chat.apply({ kind: "text", delta: `**回答** ${i} ${"x".repeat(size)}` });
    chat.apply({ kind: "settled", aborted: false });
  }
  return chat;
}

test("one document per finished message; only a changed message gets a new one", async () => {
  const chat = conversation(3);
  const { docs, made } = counted();
  await docs.sync(chat.rows);
  assert.equal(made.filter((t) => t).length, 3);
  await docs.sync(chat.rows);
  assert.equal(made.filter((t) => t).length, 3, "nothing changed, nothing made");
  chat.user("再问");
  chat.begin();
  chat.apply({ kind: "text", delta: "流" });
  await docs.sync(chat.rows);
  chat.apply({ kind: "text", delta: "式" });
  await docs.sync(chat.rows);
  assert.deepEqual(made.filter((t) => t).slice(3), ["流", "流式"], "only the streaming message");
  const rows = docs.props(chat.rows);
  const keys = JSON.stringify(Object.keys(rows[0]).sort());
  for (const row of rows) assert.equal(JSON.stringify(Object.keys(row).sort()), keys, "every row has the same shape");
  const assistant = rows.filter((r) => r.kind === "assistant");
  assert.deepEqual(assistant.map((r) => r.body.text), ["**回答** 0 xxxxxxxxxx", "**回答** 1 xxxxxxxxxx", "**回答** 2 xxxxxxxxxx", "流式"]);
  assert.ok(assistant.every((r) => r.text === ""), "the document carries the text, not a second copy");
  assert.ok(rows.filter((r) => r.kind !== "assistant").every((r) => r.body.text === ""));
});

test("the value stays inside the player's limits: newest rows first, a notice for the rest", async () => {
  // 400 rows (> MAX_ROWS) of 4 KB, and one message larger than a document may be.
  const chat = conversation(200, 4000);
  chat.user("大");
  chat.begin();
  chat.apply({ kind: "text", delta: "y".repeat(300 * 1024) });
  chat.apply({ kind: "settled", aborted: false });
  const { docs } = counted();
  await docs.sync(chat.rows);
  const rows = docs.props(chat.rows);
  assert.ok(rows.length <= MAX_ROWS + 1);
  assert.equal(rows[0].kind, "notice");
  assert.match(rows[0].text, /^更早的 \d+ 条消息没有显示/);
  assert.equal(rows.at(-1).kind, "assistant");
  assert.ok(Buffer.byteLength(rows.at(-1).body.text) <= MAX_DOC_BYTES);
  assert.match(rows.at(-1).body.text, /内容过长/);
  let total = 0;
  for (const row of rows) {
    for (const field of [row.text, row.detail]) {
      assert.ok(Buffer.byteLength(field) <= MAX_FIELD_BYTES);
      total += Buffer.byteLength(field);
    }
    total += Buffer.byteLength(row.body.text);
  }
  assert.ok(total <= MAX_TEXT_BYTES, `${total} bytes`);
});

test("in the window: Markdown body, streamed document, code copy, links, no remote images", async () => {
  const reply = "## 标题\n\n![pixel](https://tracker.example/p.png)\n\n```js\nconsole.log(1)\n```\n\n[pi](https://github.com/earendil-works/pi)";
  const mock = await server(() => ({ text: [reply.slice(0, 10), reply.slice(10)], pauseMs: 400 }));
  const ai = await startAi({ compat: mock.url });
  ai.host.remoteImages.set("https://tracker.example/p.png", null);
  await ai.ask("Markdown 请", { wait: false });
  // The streaming message already shows as a document.
  await ai.until(() => ai.props.busy === true && ai.host.markdown(ai.props.rows.at(-1)?.body)?.text === reply.slice(0, 10), "first part as Markdown", 10_000);
  await ai.until(() => ai.props.busy === false, "end");
  const doc = ai.host.markdown(ai.props.rows.at(-1).body);
  assert.equal(doc.text, reply);
  assert.deepEqual(doc.images, {}, "remote images are never fetched");
  assert.equal(ai.props.rows.at(-1).text, "");

  await ai.send("copy-code", "console.log(1)\n");
  assert.equal(ai.host.clipboard.text, "console.log(1)\n");
  await ai.send("open-link", "https://github.com/earendil-works/pi");
  await ai.send("open-link", "javascript:alert(1)");
  assert.deepEqual(ai.host.system.urls, ["https://github.com/earendil-works/pi"]);
  await ai.stop();
});
