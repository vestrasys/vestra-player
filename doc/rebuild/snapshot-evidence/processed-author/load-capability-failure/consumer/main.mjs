export const activate=async function activate(host) {
  const fs = await import("node:fs/promises"),
    path = await import("node:path"),
    { createHash } = await import("node:crypto"),
    assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_MEDIA_PROCESSED_SMOKE;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  let source;
  async function open(name) {
    const result = await host.source.submit({
      kind: "open",
      source: path.join(work, name + ".mkv"),
      startPaused: true,
    });
    for (let i = 0; i < 150; i++) {
      source = await host.source.state();
      if (
        source.phase === "ready" &&
        source.stamp.generation === result.ticket.stamp.generation
      ) {
        await wait(150);
        return;
      }
      await wait(20);
    }
    throw Error("source ready timeout");
  }
  try {
    await open("a");
    await host.processing.replace({
      stage: "video",
      resourceId: "eq",
      revision: 1,
      expectedGeneration: source.stamp.generation,
      nodes: [
        { kind: "eq", brightness: 0.15, contrast: 1, saturation: 1, gamma: 1 },
        { kind: "gblur", sigma: 0.8, steps: 3 },
      ],
    });
    const bytes = Buffer.from(
      "1\n00:00:00,000 --> 00:00:04,000\n中文 literal <b>TEXT</b> {\\b1} \\N\n\n",
    );
    let sub = await host.subtitles.state();
    await host.subtitles.begin(1, source.stamp.generation, bytes.length);
    await host.subtitles.append(1, 0, bytes.toString("base64"));
    await host.subtitles.commit({
      operationId: 1,
      uploadId: 1,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      resourceId: "cue",
      revision: 1,
      expected: sub.selection.stamp,
    });
    sub = await host.subtitles.state();
    await host.subtitles.style({
      operationId: 2,
      revision: 1,
      expected: sub.selection.stamp,
      style: {
        fontId: sub.presentation.fonts[0].id,
        fontSize: 72,
        color: "yellow",
        outline: 2,
        marginPercent: 5,
        offsetMs: 0,
      },
    });
    sub = await host.subtitles.state();
    const processing = await host.processing.state();
    source = await host.source.state();
    const options = {
      operationId: 1,
      mode: "processed",
      expected: source.stamp,
      time: 1,
      expectedProcessingRevision: processing.configurationRevision,
      expectedSubtitle: {
        selection: sub.selection.stamp,
        presentationRevision: sub.presentation.revision,
      },
    };
    const snapshot = await host.snapshot.capture(options);
    assert.equal(snapshot.kind, "processed");
    assert.equal(snapshot.subtitle.coverageValidated, true);
    assert.deepEqual(await host.snapshot.capture(options), snapshot);
    await host.processing.replace({
      stage: "video",
      resourceId: "eq",
      revision: 2,
      expectedGeneration: source.stamp.generation,
      nodes: [],
    });
    await host.subtitles.style({
      operationId: 3,
      revision: 2,
      expected: sub.selection.stamp,
      style: null,
    });
    await open("b");
    const destination = path.join(work, "processed.png");
    const job = await host.jobs.start({
      operationId: 2,
      snapshotId: snapshot.snapshotId,
      kind: "processedPng",
      destination,
    });
    await host.snapshot.release(snapshot.snapshotId, 3);
    let terminal;
    for (let i = 0; i < 200; i++) {
      terminal = await host.jobs.state(job.jobId);
      if (["failed", "completed", "cancelled"].includes(terminal.status)) break;
      await wait(20);
    }
    assert.equal(terminal.status, "completed", JSON.stringify(terminal));
    await fs.writeFile(
      path.join(work, "consumer-result.json"),
      JSON.stringify(
        { pass: true, snapshot, terminal, after: await host.source.state() },
        null,
        2,
      ),
    );
    console.error("MEDIA_PROCESSED_SMOKE_DONE");
  } catch (error) {
    await fs.writeFile(
      path.join(work, "consumer-failure.json"),
      JSON.stringify({ error: String(error), stack: error.stack }, null, 2),
    );
    throw error;
  }
};
