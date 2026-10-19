# Producer startup: prepare, prefill, activate

Current development-2026-10-18.1 usage clarification, from current producer.rs ready/activate implementation. This changes no backend API.

1. Freeze the source stamp for the user's intended replacement. Prepare and connect the producer. Initial epoch is 1, base sample 0, and audio is paused while prepared.
2. Send a real RGBA frame at sampleTime 0 and contiguous real stereo PCM starting at sampleTime 0. Before openProvider, the host requires at least 1024 received audio frames, a same-epoch video frame whose time is <= consumedFrames, a device-acknowledged epoch, and healthy output (ok/recoverableXrun/deviceChanged). A practical prefill is 2048–2400 frames, within the 4800-frame packet limit and 9600-frame queue capacity. Await each binary write.
3. Poll producer.state until epoch matches, acknowledged is true, error is null, receivedFrames >=1024 and hasVideo is true. Initially consumedFrames is 0, so a video stamped only at the end of prefill is insufficient. The initial acknowledgement is the audio callback observing the prepared reset; it is not a producer.ack call.
4. Submit source.openProvider with the frozen expected source stamp. Query its operation on pending/unknown; do not resubmit under a new ID. Activation preserves epoch and prefilled buffers, changing the same-epoch paused state. Continue the actual producer clock from the prefilled sample position and sequence.
5. Handle pendingControl while waiting for source operations. Later controls carry epoch/baseSample/paused and are acknowledged through producer.ack after the plugin has applied them. A new epoch invalidates old data. consumedFrames is an absolute sample cursor initialized from baseSample, advancing only for matching PCM, not wall clock or silence.

```ts
const expected = (await host.source.state()).stamp;
const prepared = await host.producer.prepare({width, height});
const writer = await host.producer.connect(prepared);
await writer.video({epoch: prepared.epoch, sampleTime: 0, sequence: 0}, firstRgba);
// initialPcm contains the source's real audio, at least 1024 stereo frames.
await writer.audio({epoch: prepared.epoch, sampleTime: 0, sequence: 1}, initialPcm);
// Poll acknowledged + healthy + receivedFrames>=1024 + hasVideo, with cancellation/deadline.
const operation = await host.source.submit({kind: 'openProvider', producerId: prepared.producerId, startPaused: false, expected});
// Poll operation and process pendingControl concurrently; continue from initialPcm.length/2.
```

The snippet illustrates ordering, not a complete cancellation/cleanup loop. Do not generate fake silence or a fake image to label source readiness. This sequence is common producer infrastructure; ROM parsing, emulator state and frame/audio generation remain plugin business.
