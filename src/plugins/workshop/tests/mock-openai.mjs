// A local model server for tests, on 127.0.0.1: OpenAI chat completions
// (POST …/chat/completions) and Anthropic messages (POST …/v1/messages), both
// streamed and scripted, and the model lists (GET …/models, GET …/v1/models).
// No paid service is ever called.
import http from "node:http";

// The Fetch standard's "bad ports" (https://fetch.spec.whatwg.org/#port-blocking):
// fetch() refuses them before connecting ("bad port"). An ephemeral port can be
// one of them where the dynamic range starts low (Windows can be set to
// 1024–15000), so a server on port 0 would fail now and then.
const BAD_PORTS = new Set([
  0, 1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 139, 143,
  161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556, 563, 587, 601, 636, 989, 990, 993, 995, 1719, 1720, 1723, 2049, 3659, 4045,
  4190, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6679, 6697, 10080,
]);

/** Listens on 127.0.0.1 on a free port that fetch() accepts. */
export async function listenFetchable(server) {
  for (;;) {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    if (!BAD_PORTS.has(server.address().port)) return;
    await new Promise((resolve) => server.close(resolve));
  }
}

/**
 * `reply(request, index)` → one of
 *   { text: ["chunk", …], toolCalls?: [{ name, args }], thinking?: ["chunk", …], pauseMs?, delayMs? }
 *     (thinking: a thinking block first, Anthropic's thinking_delta or OpenAI's reasoning_content)
 *     (delayMs: before the reply starts; pauseMs: between text chunks)
 *   { status: 401, error: "Incorrect API key provided" }
 *   { hang: true }   (never answers; for stop and timeout tests)
 */
export async function mockOpenAI(reply, { models = ["mock-1", "mock-2"] } = {}) {
  const requests = [];
  const sockets = new Set();
  const server = http.createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    const request = { method: req.method, url: req.url, headers: req.headers, body: body ? JSON.parse(body) : null };
    requests.push(request);
    const route = new URL(req.url, "http://localhost").pathname;
    if (req.method === "GET" && route.endsWith("/models")) {
      if (!models) {
        res.writeHead(404).end();
        return;
      }
      res.writeHead(200, { "content-type": "application/json" });
      res.end(JSON.stringify({ object: "list", data: models.map((id) => ({ id, object: "model", type: "model", display_name: id })) }));
      return;
    }
    const anthropic = req.method === "POST" && route.endsWith("/v1/messages");
    if (!anthropic && (req.method !== "POST" || !route.endsWith("/chat/completions"))) {
      res.writeHead(404).end();
      return;
    }
    const answer = await reply(request, requests.length - 1);
    if (answer.hang) return;
    if (answer.delayMs) await new Promise((resolve) => setTimeout(resolve, answer.delayMs));
    if (answer.status) {
      res.writeHead(answer.status, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: { message: answer.error, type: "invalid_request_error", code: answer.code ?? null } }));
      return;
    }
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
    if (anthropic) {
      const send = (type, data) => res.write(`event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`);
      send("message_start", { message: { id: "msg_test", type: "message", role: "assistant", model: request.body.model, content: [], stop_reason: null, usage: { input_tokens: 1, output_tokens: 0 } } });
      let index = 0;
      if (answer.thinking?.length) {
        send("content_block_start", { index, content_block: { type: "thinking", thinking: "" } });
        for (const thinking of answer.thinking) send("content_block_delta", { index, delta: { type: "thinking_delta", thinking } });
        send("content_block_delta", { index, delta: { type: "signature_delta", signature: "sig-test" } });
        send("content_block_stop", { index: index++ });
      }
      if (answer.text?.length) {
        send("content_block_start", { index, content_block: { type: "text", text: "" } });
        for (const text of answer.text) send("content_block_delta", { index, delta: { type: "text_delta", text } });
        send("content_block_stop", { index: index++ });
      }
      for (const call of answer.toolCalls ?? []) {
        send("content_block_start", { index, content_block: { type: "tool_use", id: `toolu_${requests.length}_${index}`, name: call.name, input: {} } });
        send("content_block_delta", { index, delta: { type: "input_json_delta", partial_json: JSON.stringify(call.args) } });
        send("content_block_stop", { index: index++ });
      }
      send("message_delta", { delta: { stop_reason: answer.toolCalls?.length ? "tool_use" : "end_turn" }, usage: { output_tokens: 1 } });
      send("message_stop", {});
      res.end();
      return;
    }
    const chunk = (delta, finish = null) =>
      res.write(
        `data: ${JSON.stringify({ id: "chatcmpl-test", object: "chat.completion.chunk", created: 0, model: request.body.model, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`,
      );
    chunk({ role: "assistant", content: "" });
    for (const reasoning of answer.thinking ?? []) chunk({ reasoning_content: reasoning });
    for (const text of answer.text ?? []) {
      chunk({ content: text });
      if (answer.pauseMs) await new Promise((resolve) => setTimeout(resolve, answer.pauseMs));
    }
    for (const [index, call] of (answer.toolCalls ?? []).entries())
      chunk({ tool_calls: [{ index, id: `call_${requests.length}_${index}`, type: "function", function: { name: call.name, arguments: JSON.stringify(call.args) } }] });
    chunk({}, answer.toolCalls?.length ? "tool_calls" : "stop");
    res.write("data: [DONE]\n\n");
    res.end();
  });
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("close", () => sockets.delete(socket));
  });
  await listenFetchable(server);
  return {
    url: `http://127.0.0.1:${server.address().port}/v1`,
    requests,
    /** The model calls only (not the model list requests). */
    get chats() {
      return requests.filter((r) => r.method === "POST");
    },
    async close() {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}
