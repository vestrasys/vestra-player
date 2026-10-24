// A local OpenAI-compatible chat completions server for tests: scripted,
// streamed replies on 127.0.0.1. No paid service is ever called.
import http from "node:http";

/**
 * `reply(request, index)` → one of
 *   { text: ["chunk", …], toolCalls?: [{ name, args }], pauseMs? }
 *   { status: 401, error: "Incorrect API key provided" }
 *   { hang: true }   (never answers; for stop and timeout tests)
 */
export async function mockOpenAI(reply) {
  const requests = [];
  const sockets = new Set();
  const server = http.createServer(async (req, res) => {
    let body = "";
    for await (const chunk of req) body += chunk;
    const request = { method: req.method, url: req.url, headers: req.headers, body: body ? JSON.parse(body) : null };
    requests.push(request);
    if (req.method !== "POST" || !req.url.endsWith("/chat/completions")) {
      res.writeHead(404).end();
      return;
    }
    const answer = await reply(request, requests.length - 1);
    if (answer.hang) return;
    if (answer.status) {
      res.writeHead(answer.status, { "content-type": "application/json" });
      res.end(JSON.stringify({ error: { message: answer.error, type: "invalid_request_error", code: answer.code ?? null } }));
      return;
    }
    res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
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
    async close() {
      for (const socket of sockets) socket.destroy();
      await new Promise((resolve) => server.close(resolve));
    },
  };
}
