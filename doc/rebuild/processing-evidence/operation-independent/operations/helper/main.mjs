export const activate=async function helper(host) {
  const fs = await import("node:fs/promises"), path = await import("node:path");
  const p = host.processing, token = { stage: "video", resourceId: "live", revision: 1 };
  const prior = await p.operation(token), s = await host.source.state();
  const result = await p.replace({ ...token, expectedGeneration: s.stamp.generation, nodes: [{ kind: "eq", brightness: -.03, contrast: 1, saturation: 1, gamma: 1 }] });
  const state = await p.state(), owner = state.video.find(r => r.resource.manifestId === "review.processing-operation-helper").owner;
  const query = await p.operation(token);
  await fs.appendFile(path.join(process.env.VESTRA_PROCESSING_OPERATION_REVIEW, "helper-results.jsonl"), JSON.stringify({ owner, prior, result, query }) + "\n");
  // Only expose the public ready marker after writing the independent evidence.
  await p.replace({ ...token, resourceId: "ready", expectedGeneration: s.stamp.generation, nodes: [{ kind: "eq", brightness: 0, contrast: 1, saturation: 1, gamma: 1 }] });
};
