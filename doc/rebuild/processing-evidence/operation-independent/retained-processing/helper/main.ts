const wire=async function wire() {
  const { createInterface } = await import("node:readline");
  const input = createInterface({ input: process.stdin });
  let next = 1000000;
  const pending = new Map();
  input.on("line", line => {
    const message = JSON.parse(line), entry = pending.get(message.id);
    if (!entry) return;
    pending.delete(message.id); clearTimeout(entry.timer);
    if (message.error) entry.reject(Object.assign(new Error(message.error.message), message.error));
    else entry.resolve(message.result);
  });
  return (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++next;
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`${method} timed out; outcome unknown`)); }, 8000);
    pending.set(id, { resolve, reject, timer });
    process.stdout.write(JSON.stringify({ id, method, params }) + "\n");
  });
};
export const activate=async function helper(host) {
  const request = await wire(), s = await host.source.state();
  for (const [stage, nodes] of [
    ["video", [{ kind: "eq", brightness: -0.05, contrast: 1, saturation: 1, gamma: 1 }]],
    ["audio", [{ kind: "volume", gainDb: -3 }]],
  ]) await request("processing.replace", { stage, resourceId: "same-local-id", revision: 1, expectedGeneration: s.stamp.generation, nodes });
};
