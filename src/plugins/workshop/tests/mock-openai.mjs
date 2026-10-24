// A local model server for tests, on 127.0.0.1: OpenAI chat completions
// (POST …/chat/completions) and Anthropic messages (POST …/v1/messages), both
// streamed and scripted, and the model lists (GET …/models, GET …/v1/models).
// No paid service is ever called.
import http from "node:http";

/**
 * `reply(request, index)` → one of
 *   { text: ["chunk", …], toolCalls?: [{ name, args }], pauseMs?, delayMs? }
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
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
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
