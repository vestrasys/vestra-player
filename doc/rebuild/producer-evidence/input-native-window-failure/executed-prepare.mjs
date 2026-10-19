/** Native-input author probe. Default prepares only; --run is an explicit later GUI/device run. */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, writeFile, cp } from "node:fs/promises";
import { resolve } from "node:path";

const work = await mkdtemp(resolve("/tmp", "vestra-producer-input-native-"));
const consumer = resolve(work, "consumer");
const dependencies = resolve(work, "dependencies");
await mkdir(consumer); await mkdir(dependencies);
const archivePath = resolve(import.meta.dirname, "../../doc/rebuild/producer-evidence/nes-core-prerequisite/jsnes-2.1.0.tgz");
const romPath = resolve(import.meta.dirname, "../../doc/rebuild/producer-evidence/nes-fixture-preparation/thwaite.nes");
const archive = await readFile(archivePath);
const rom = await readFile(romPath);
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const archiveSha = "167081846a907e19bdd7cba75c648d827d49ea50e0fb58c079ad1a6613dee3c1";
const romSha = "ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173";
assert.ok(archive.length <= 4 * 1024 * 1024);
assert.equal(sha(archive), archiveSha); assert.equal(sha(rom), romSha);
await writeFile(resolve(work, "jsnes-2.1.0.tgz"), archive);
await writeFile(resolve(work, "thwaite.nes"), rom);
// Recreate this run's dependency; do not use/update an archived preparation's
// old temporary path. No npm install, package scripts, symlinks or archive paths
// outside the explicitly allowed official source/metadata subset are extracted.
const extraction = `
from pathlib import Path, PurePosixPath
import sys, tarfile, hashlib, json
archive, destination = Path(sys.argv[1]), Path(sys.argv[2])
assert hashlib.sha256(archive.read_bytes()).hexdigest() == '${archiveSha}'
seen, total, count = set(), 0, 0
with tarfile.open(archive, 'r:gz') as source:
 for member in source:
  path = PurePosixPath(member.name)
  assert not path.is_absolute() and path.parts and path.parts[0] == 'package' and '..' not in path.parts
  assert not member.issym() and not member.islnk() and not member.isdev()
  if member.isdir(): continue
  assert member.isfile()
  if not (member.name.startswith('package/src/') or member.name in ('package/package.json','package/LICENSE','package/README.md','package/index.d.ts')): continue
  assert member.name not in seen
  seen.add(member.name)
  assert 0 <= member.size <= 2*1024*1024
  total += member.size; count += 1
  assert total <= 8*1024*1024 and count <= 256
  data = source.extractfile(member).read(2*1024*1024+1)
  assert len(data) == member.size
  target = destination.joinpath(*path.parts)
  target.parent.mkdir(parents=True, exist_ok=True)
  with target.open('xb') as output: output.write(data)
metadata = json.loads((destination/'package/package.json').read_text())
assert metadata['name'] == 'jsnes' and metadata['version'] == '2.1.0'
print(json.dumps({'files': count, 'bytes': total, 'name': metadata['name'], 'version': metadata['version']}))
`;
await writeFile(resolve(work, "extract.py"), extraction);
const extracted = spawnSync("python3", [resolve(work, "extract.py"), resolve(work, "jsnes-2.1.0.tgz"), dependencies], { encoding: "utf8", timeout: 20000, maxBuffer: 1024 * 1024 });
await writeFile(resolve(work, "extract.stdout.log"), extracted.stdout ?? "");
await writeFile(resolve(work, "extract.stderr.log"), extracted.stderr ?? "");
await writeFile(resolve(work, "preparation.json"), JSON.stringify({ archivePath, romPath, archiveSha, romSha, archiveBytes: archive.length, romBytes: rom.length, extraction: { exitCode: extracted.status, signal: extracted.signal, error: String(extracted.error ?? "") } }, null, 2));
assert.equal(extracted.status, 0, extracted.stderr || String(extracted.error));

const run = process.argv.includes("--run");
assert.ok(!(run && process.argv.includes("--prepare-only")), "choose --run or --prepare-only");
const durationIndex = process.argv.indexOf("--duration");
const duration = durationIndex < 0 ? 120 : Number(process.argv[durationIndex + 1]);
assert.ok(Number.isInteger(duration) && duration >= 5 && duration <= 120, "duration must be5..120 seconds");
const bundle=resolve(work,"Vestra Producer Input.app");
const resources=resolve(bundle,"Contents/Resources");
const bundleId=`local.vestra.rebuild.producer-input-${sha(Buffer.from(work)).slice(0,12)}`;
await writeFile(resolve(work, "config.json"), JSON.stringify({ duration, assetRoot:resources, stopFile: resolve(work, "stop"), withholdFile: resolve(work, "withhold-input-ack") }, null, 2));

async function activate(host) {
  const fs = await import("node:fs/promises");
  const path = await import("node:path");
  const { pathToFileURL } = await import("node:url");
  const { createHash } = await import("node:crypto");
  const assert = (await import("node:assert/strict")).default;
  const work = process.env.VESTRA_PRODUCER_INPUT_NATIVE;
  const config = JSON.parse(await fs.readFile(path.join(work, "config.json"), "utf8"));
  const logPath = path.join(work, "events.jsonl");
  let logBytes = 0;
  async function log(kind, data) {
    const line = JSON.stringify({ at: Date.now(), kind, ...data }) + "\n";
    logBytes += Buffer.byteLength(line);
    assert.ok(logBytes <= 16 * 1024 * 1024, "bounded raw event evidence exhausted");
    await fs.appendFile(logPath, line);
  }
  async function exists(file) {
    try { await fs.access(file); return true; }
    catch (error) { if (error.code === "ENOENT") return false; throw error; }
  }
  const wait = (ms) => new Promise((done) => setTimeout(done, ms));
  const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
  let connection, prepared, sourceTicket;
  let generatedFrames = 0, generatedAudioCursor = 0, frameEvents = 0;
  let frameSamples = 0, epoch = 1, sendCursor = 0, sequence = 0, operation = 0;
  let peak = 0, nonzeroFrames = 0, appliedReceipts = 0, replayedReceipts = 0;
  let lastReceipt, lastReceiptNotice, lastScope, withholdReceipt;
  let lastHealth, warmup, outcome = "failed", failure, latestState, latestSource;
  const pressedButtons = new Set();
  const controls = [], checkpoints = [];
  const menuRequests=[];
  let menuOperation,menuFault,commandRevision=1,unlisten;
  async function commandState(enabled,busy,paused,reason=null) {
    return host.commands.update("togglePause",++commandRevision,{enabled,checked:paused,busy,reason});
  }
  const audioScratch = new Float32Array(4800 * 2);
  const pixels = new Uint8Array(256 * 240 * 4);
  const pcmHash = createHash("sha256"), videoHash = createHash("sha256");
  const sampleBytes = Buffer.alloc(8);
  let core, Controller;
  function coreFacts() {
    return { generatedFrames, generatedAudioCursor, frameEvents, frameSamples,
      peak, nonzeroFrames, epoch, sendCursor, pressedButtons: [...pressedButtons].sort((a,b) => a-b),
      lastFrameSha256: sha(pixels), pcmSha256: pcmHash.copy().digest("hex"),
      frameSequenceSha256: videoHash.copy().digest("hex") };
  }
  function releaseAll(reason) {
    const released = [...pressedButtons];
    if (core) for (let button = 0; button < 8; button++) core.buttonUp(1, button);
    pressedButtons.clear();
    return { reason, released, generatedFrames, generatedAudioCursor };
  }
  function step() {
    frameSamples = 0;
    const before = frameEvents, audioStart = generatedAudioCursor;
    core.frame(); generatedFrames++;
    assert.equal(frameEvents, before + 1);
    assert.ok(frameSamples > 0 && frameSamples <= 1024, "fixed core frame left bounded audio profile");
    return { samples: audioScratch.slice(0, frameSamples * 2), count: frameSamples, audioStart };
  }
  async function state() {
    const current = await host.producer.state(prepared.producerId);
    latestState = current;
    const health = { health: current.health, deviceErrors: current.deviceErrors,
      xrunEvents: current.xrunEvents, deviceChangeEvents: current.deviceChangeEvents,
      errorDetails: current.errorDetails, errorOverflow: current.errorOverflow, buffer: current.buffer };
    if (JSON.stringify(health) !== JSON.stringify(lastHealth)) {
      lastHealth = health; await log("audio-health", health);
    }
    assert.ok(["ok", "recoverableXrun", "deviceChanged"].includes(current.health), JSON.stringify(current));
    assert.equal(current.error, null, JSON.stringify(current));
    return current;
  }
  async function until(label, read, predicate, milliseconds = 3000) {
    const deadline = Date.now() + milliseconds;
    let value;
    do {
      value = await read();
      if (predicate(value)) return value;
      await wait(10);
    } while (Date.now() < deadline);
    throw Error(`${label}: ${JSON.stringify(value)}`);
  }
  function scopeKey(scope) {
    const t = scope.ticket;
    return JSON.stringify([scope.owner, scope.producerId, scope.inputEpoch,
      t.owner, t.cycle, t.stamp.generation, t.stamp.intentVersion]);
  }
  function applyNotice(notice) {
    assert.equal(notice.scope.producerId, prepared.producerId);
    if (!notice.captured) assert.deepEqual(notice.pressed, [], "revoke must contain an empty complete set");
    const keys = new Set(notice.captured ? notice.pressed : []);
    const desired = new Set();
    const map = { ArrowUp: Controller.BUTTON_UP, ArrowDown: Controller.BUTTON_DOWN,
      ArrowLeft: Controller.BUTTON_LEFT, ArrowRight: Controller.BUTTON_RIGHT,
      KeyX: Controller.BUTTON_A, KeyZ: Controller.BUTTON_B, Enter: Controller.BUTTON_START };
    for (const [code, button] of Object.entries(map)) if (keys.has(code)) desired.add(button);
    if (keys.has("ShiftLeft") || keys.has("ShiftRight")) desired.add(Controller.BUTTON_SELECT);
    const changes = [];
    for (let button = 0; button < 8; button++) {
      if (desired.has(button) && !pressedButtons.has(button)) { core.buttonDown(1, button); changes.push({ button, down: true }); }
      else if (!desired.has(button) && pressedButtons.has(button)) { core.buttonUp(1, button); changes.push({ button, down: false }); }
    }
    pressedButtons.clear(); for (const button of desired) pressedButtons.add(button);
    return { physical: [...keys].sort(), logical: [...desired].sort((a,b) => a-b), changes,
      selectHeldBy: ["ShiftLeft", "ShiftRight"].filter((key) => keys.has(key)) };
  }
  async function drainInputs() {
    // Each FIFO front is applied at this between-frame boundary before ACK.
    // Rapid down/up is drained in order, even when it falls within one frame.
    for (let i = 0; i < 32; i++) {
      const current = await state(), notice = current.input?.pending;
      if (!notice) return true;
      const scope = scopeKey(notice.scope), receipt = `${scope}:${notice.sequence}`;
      let mapping, replay = false;
      if (receipt === lastReceipt) {
        assert.deepEqual(notice, lastReceiptNotice, "same receipt changed contents");
        replayedReceipts++; replay = true;
      } else {
        if (scope !== lastScope) await log("scope-release", releaseAll("input scope changed"));
        else if (lastReceiptNotice) assert.ok(notice.sequence > lastReceiptNotice.sequence, "nonmonotone input receipt");
        mapping = applyNotice(notice);
        lastScope = scope; lastReceipt = receipt; lastReceiptNotice = notice; appliedReceipts++;
        await log("input-apply", { notice, mapping, core: coreFacts(), replay: false });
      }
      if (await exists(config.withholdFile)) {
        if (withholdReceipt !== receipt) {
          withholdReceipt = receipt;
          await log("input-ack-deliberately-withheld", { notice, mapping, core: coreFacts(),
            note: "explicit marker only; core does not advance while this FIFO front remains unacknowledged" });
        }
        return false;
      }
      const ack = await host.producer.ackInput(notice);
      assert.equal(ack.status, "acknowledged"); assert.equal(ack.sequence, notice.sequence);
      await log("input-ack", { notice, mapping: mapping ?? null, replay, businessReapplied: false, ack, coreFrame: generatedFrames });
    }
    // Never step a core frame if the bounded drain could still have a front.
    const remaining = (await state()).input?.pending;
    if (remaining) await log("input-drain-yield", { sequence: remaining.sequence, coreFrame: generatedFrames });
    return !remaining;
  }
  async function control(notice) {
    const boundary = coreFacts(), released = releaseAll("source pause/resume control");
    const oldCursor = sendCursor;
    if (notice.paused) {
      assert.ok(notice.baseSample <= oldCursor && oldCursor - notice.baseSample <= 4096);
      epoch = notice.epoch; sendCursor = notice.baseSample;
      frameSamples = 0; audioScratch.fill(0);
    } else {
      assert.equal(notice.epoch, epoch);
      assert.equal(notice.baseSample, sendCursor);
    }
    const ack = await host.producer.ack(notice);
    await log("control-provider-ack", { notice, boundary, released, oldCursor,
      discardedQueuedAudioFrames: notice.paused ? oldCursor - notice.baseSample : 0, ack,
      note: "between NES.frame calls; not a zero-drop or exact-consumed-frame checkpoint" });
    assert.equal(ack.status, "acknowledged"); assert.deepEqual(ack.control, notice);
    const deadline = Date.now() + 3000;
    let confirmed;
    do {
      // Revoke notices still need timely delivery while this core is frozen.
      await drainInputs();
      confirmed = await state();
      if (!confirmed.pendingControl && confirmed.acknowledged && confirmed.paused === notice.paused && confirmed.epoch === notice.epoch) break;
      await wait(10);
    } while (Date.now() < deadline);
    assert.ok(!confirmed.pendingControl && confirmed.acknowledged && confirmed.paused === notice.paused && confirmed.epoch === notice.epoch, "control did not settle");
    assert.equal(generatedFrames, boundary.generatedFrames);
    controls.push({ notice, boundary, confirmed, generatedFrames });
    assert.ok(controls.length <= 256);
    await log("control-confirmed", { notice, state: confirmed, core: coreFacts(),
      note: "original requester's source.operation must be checked by its controller; provider does not query another owner's operation ID" });
  }
  async function emit(consumed) {
    assert.ok(sendCursor >= consumed && sendCursor - consumed <= 3072);
    const frame = step(), sampleTime = sendCursor;
    assert.ok(sendCursor + frame.count - consumed <= 4096);
    await connection.video({ epoch, sampleTime, sequence: sequence++ }, pixels);
    await connection.audio({ epoch, sampleTime, sequence: sequence++ }, frame.samples);
    sendCursor += frame.count;
  }
  try {
    const module = await import(pathToFileURL(path.join(config.assetRoot, "dependencies/package/src/index.js")).href);
    Controller = module.Controller;
    const rom = await fs.readFile(path.join(config.assetRoot, "thwaite.nes"));
    assert.equal(sha(rom), "ee51cd9562f28195ba015d9857c6c4fc9bf67cdfb213e95f655e586b92195173");
    core = new module.NES({ sampleRate: 48000,
      onFrame(words) {
        assert.equal(words.length, 256 * 240);
        for (let i = 0; i < words.length; i++) {
          const word = words[i]; pixels[i*4] = word & 255; pixels[i*4+1] = (word >>> 8) & 255;
          pixels[i*4+2] = (word >>> 16) & 255; pixels[i*4+3] = 255;
        }
        frameEvents++; videoHash.update(pixels);
      },
      onAudioSample(left, right) {
        assert.ok(Number.isFinite(left) && Number.isFinite(right) && Math.abs(left) <= 1 && Math.abs(right) <= 1);
        assert.ok(frameSamples < 4800);
        audioScratch[frameSamples*2] = left; audioScratch[frameSamples*2+1] = right;
        frameSamples++; generatedAudioCursor++;
        peak = Math.max(peak, Math.abs(left), Math.abs(right)); if (left || right) nonzeroFrames++;
        sampleBytes.writeFloatLE(left,0); sampleBytes.writeFloatLE(right,4); pcmHash.update(sampleBytes);
      },
    });
    core.loadROM(new Uint8Array(rom));
    // Fixed prereq warmup only; these are direct core inputs, never synthetic
    // host/window input injections. Actual probe interaction comes from CUA.
    for (let i = 0; i < 180; i++) step();
    for (let phase = 0; phase < 2; phase++) {
      core.buttonDown(1,Controller.BUTTON_START); for (let i=0;i<2;i++) step();
      core.buttonUp(1,Controller.BUTTON_START); for (let i=0;i<178;i++) step();
    }
    releaseAll("warmup end"); warmup = coreFacts();
    await fs.writeFile(path.join(work,"warmup-game.rgba"),pixels);
    await log("warmup", { core: warmup, sequence: "180 frames; Start2/up178; Start2/up178", channelOrder: "low-byte R/G/B, A255" });
    // Ordinary public command, registered before AV activation/input declaration.
    // No source input is captured until CUA explicitly clicks the ready canvas.
    unlisten=host.commands.onExecute((event)=>{
      if(event.id!=="togglePause") return;
      if(menuRequests.length>=8) {menuFault=Error("bounded public command queue exhausted");return;}
      menuRequests.push(event);
    });
    await host.commands.register("togglePause",commandRevision,
      {title:"暂停/继续（实验）",path:["播放","暂停/继续（实验）"],shortcut:null},
      {enabled:false,checked:false,busy:false,reason:"实验来源准备中"});
    prepared = await host.producer.prepare({width:256,height:240});
    assert.equal(prepared.sampleRate,48000); assert.equal(prepared.epoch,1);
    connection = await host.producer.connect(prepared);
    await until("candidate callback",state,(s)=>s.acknowledged && s.paused);
    while(sendCursor<2048) await emit(0);
    const ready = await until("candidate first AV",state,(s)=>s.acknowledged && s.hasVideo && s.receivedFrames>=1024);
    latestSource = await host.source.state();
    const opened = await host.source.submit({kind:"openProvider",producerId:prepared.producerId,startPaused:false,expected:latestSource.stamp},++operation);
    assert.notEqual(opened.status,"failed",JSON.stringify(opened)); sourceTicket=opened.ticket;
    await until("source ready",()=>host.source.state(),(s)=>s.phase==="ready" && s.stamp.generation===sourceTicket.stamp.generation);
    const keys=["ArrowUp","ArrowDown","ArrowLeft","ArrowRight","KeyX","KeyZ","Enter","ShiftLeft","ShiftRight"];
    const declared = await host.producer.declareInput(prepared.producerId,keys);
    await log("input-declared",{keys,declared,ready,sourceTicket});
    await until("producer running",state,(s)=>s.active && s.acknowledged && !s.paused);
    await commandState(true,false,false);
    await fs.writeFile(path.join(work,"ready.json"),JSON.stringify({prepared,sourceTicket,keys,work,config},null,2));
    console.error("PRODUCER_INPUT_NATIVE_READY",work);
    const started=Date.now(); let lastCheckpoint=0,lastStopCheck=0;
    while(Date.now()-started<config.duration*1000) {
      if(Date.now()-lastStopCheck>=100) {
        lastStopCheck=Date.now();
        if(await exists(config.stopFile)) {outcome="stopped";break;}
      }
      let current=await state();
      if(current.pendingControl) {await control(current.pendingControl);continue;}
      const drained=await drainInputs();
      current=await state();
      if(current.pendingControl) continue;
      if(current.input?.pending && drained) continue;
      if(menuFault) throw menuFault;
      if(menuOperation) {
        const result=await host.source.operation(menuOperation.id);
        if(result.status!=="pending") {
          await log("public-command-operation-terminal",{request:menuOperation,result,core:coreFacts()});
          assert.equal(result.status,"completed",JSON.stringify(result));
          await commandState(true,false,current.paused);
          menuOperation=undefined;
        }
      }
      if(!menuOperation && menuRequests.length) {
        const event=menuRequests.shift();
        const source=await host.source.state();
        assert.equal(source.stamp.generation,sourceTicket.stamp.generation,"public command source changed");
        const intent={kind:"control",control:{kind:"pause",paused:!source.paused},expected:source.stamp};
        const id=++operation;
        await commandState(true,true,source.paused,"等待帧边界确认");
        const accepted=await host.source.submit(intent,id);
        assert.notEqual(accepted.status,"failed",JSON.stringify(accepted));
        menuOperation={id,intent,event,accepted};
        await log("public-command-submitted",{request:menuOperation,core:coreFacts()});
        continue;
      }
      if(Date.now()-lastCheckpoint>=1000) {
        lastCheckpoint=Date.now(); latestSource=await host.source.state();
        const checkpoint={elapsedMs:Date.now()-started,core:coreFacts(),producer:current,source:latestSource,appliedReceipts,replayedReceipts};
        checkpoints.push(checkpoint); assert.ok(checkpoints.length<=130);
        await log("checkpoint",checkpoint);
        await fs.writeFile(path.join(work,"latest.json"),JSON.stringify(checkpoint,null,2));
        if(latestSource.phase==="error") {outcome="sourceError";throw Error(`source error: ${JSON.stringify(latestSource)}`);}
        if(latestSource.stamp.generation!==sourceTicket.stamp.generation || latestSource.phase==="empty") {outcome="sourceChanged";break;}
      }
      if(drained && !current.paused && sendCursor-current.consumedFrames<=3072) {
        assert.equal(current.epoch,epoch); await emit(current.consumedFrames);
      } else await wait(8);
    }
    if(outcome==="failed") outcome="durationEnded";
  } catch(error) {
    failure={message:String(error),code:error.code,stack:error.stack};
    try {latestSource=await host.source.state();if(latestSource.phase==="error") outcome="sourceError";} catch{}
    await log("probe-error",{failure,source:latestSource,producer:latestState,core:coreFacts(),withholdReceipt}).catch(()=>{});
  } finally {
    unlisten?.();
    const released=releaseAll("probe shutdown");
    let cleanup,closedOwnedSource=false;
    try {
      const source=await host.source.state();
      if(sourceTicket && source.stamp.generation===sourceTicket.stamp.generation && !["empty","error"].includes(source.phase)) {
        cleanup=await host.source.submit({kind:"close",expected:source.stamp},++operation);
        assert.notEqual(cleanup.status,"failed",JSON.stringify(cleanup));
        closedOwnedSource=true;
      }
      if(prepared) {
        try {
          const remaining=await host.producer.state(prepared.producerId);
          assert.equal(closedOwnedSource,false,"source.close did not remove its producer before socket cleanup");
          assert.equal(remaining.active,false,"unexpected active producer remains at probe shutdown");
          await host.producer.release(prepared.producerId);
        }
        catch(error) {if(error.code!=="unavailable") throw error;}
      }
    } catch(error) {
      cleanup={error:String(error),code:error.code};
      if(!failure) {failure={message:"probe cleanup failed",details:cleanup};outcome="cleanupFailed";}
    }
    connection?.close();
    const result={outcome,failure,prepared,sourceTicket,warmup,finalCore:coreFacts(),appliedReceipts,replayedReceipts,controls,checkpoints,latestHealth:lastHealth,latestSource,latestState,withholdReceipt,cleanup,released,
      limits:["Prepared author probe, no automatic declaration that CUA cases passed.","Only actual native keys/menu/focus events can establish the requested input observations; no host input injection exists here.","TS maps independent physical Shift keys to Select OR; host does not know NES mapping.","At most32 input FIFO fronts are applied/ACKed before each core frame; repeated receipts never repeat business apply.","Explicit withhold-input-ack marker stalls ACK/core and can trigger host timeout; that is recorded as sourceError/failure, never a pass.","Between-frame freeze may discard bounded<=4096 pre-generated PCM; no exact consumed-frame checkpoint/zero-drop promise.","No screen pixel/physical GUI/voice/10minute/save/reset/newROM acceptance inferred from this script."]};
    await fs.writeFile(path.join(work,"result.json"),JSON.stringify(result,null,2));
    console.error(failure ? "PRODUCER_INPUT_NATIVE_FAILED" : "PRODUCER_INPUT_NATIVE_STOPPED",JSON.stringify({work,outcome}));
  }
}

await writeFile(resolve(consumer,"manifest.json"),JSON.stringify({id:"author.producer-input-native",apiVersion:"vestra.experimental/0.1",entry:"main.mjs",capabilities:["producer.av","playback.read","playback.control","events","commands"]}));
await writeFile(resolve(consumer,"main.mjs"),`export const activate = ${activate.toString()};\n`);
const sourceBinary=process.env.VESTRA_HOST_BINARY ?? resolve(import.meta.dirname,"../host/target/debug/vestra-host");
await mkdir(resolve(bundle,"Contents/MacOS"),{recursive:true});
await cp(sourceBinary,resolve(bundle,"Contents/MacOS/host"));
await mkdir(resources,{recursive:true});
for(const name of ["sdk","runtime"]) await cp(resolve(import.meta.dirname,"..",name),resolve(resources,name),{recursive:true});
await cp(resolve(import.meta.dirname,"../host/ui"),resolve(resources,"ui"),{recursive:true});
await cp(consumer,resolve(resources,"consumer"),{recursive:true});
await cp(dependencies,resolve(resources,"dependencies"),{recursive:true});
await cp(resolve(work,"thwaite.nes"),resolve(resources,"thwaite.nes"));
await writeFile(resolve(bundle,"Contents/Info.plist"),`<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>host</string><key>CFBundleIdentifier</key><string>${bundleId}</string><key>CFBundleName</key><string>Vestra Producer Input</string><key>CFBundlePackageType</key><string>APPL</string><key>NSHighResolutionCapable</key><true/></dict></plist>`);
const binary=resolve(bundle,"Contents/MacOS/host");
// Native Slint window; no --headless and no fabricated input. The silent flag
// disables ordinary-file mpv audio, while the producer uses its real CPAL device.
const args=["--silent","--seconds",String(duration+15),"--plugin",resolve(resources,"consumer")];
const env={VESTRA_PRODUCER_INPUT_NATIVE:work,PATH:"/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"};
const openArgs=["-n",bundle,...Object.entries(env).flatMap(([key,value])=>["--env",`${key}=${value}`]),"--stdout",resolve(work,"host.stdout.log"),"--stderr",resolve(work,"host.stderr.log"),"--args",...args];
const launch={preparedOnly:!run,binary,sourceBinary,bundle,bundleId,args,env,work,command:["open",...openArgs],stopFile:resolve(work,"stop"),withholdFile:resolve(work,"withhold-input-ack"),
  instructions:["Wait for ready.json before physical CUA clicks/keys; bundle is self-contained and uniquely identified.","Use Playback / 暂停/继续（实验） ordinary public command for pause/resume; original requester operation is logged separately from provider ACK.","Inspect events.jsonl input-apply/input-ack plus latest.json; no screenshot claims are automatic.","Create stop file for graceful finish. Optional late-stage withhold-input-ack marker blocks subsequent ACKs; expect timeout and non-pass sourceError result.","Use the exact open -n command only after maintainer freezes host and grants exclusive device/window run. Do not getApp an unknown/nonrunning bundle to launch an argumentless copy."]};
await writeFile(resolve(work,"launch.json"),JSON.stringify(launch,null,2));
console.log(JSON.stringify(launch));
if(run) {
  const child=spawnSync("open",openArgs,{encoding:"utf8",timeout:10000,maxBuffer:1024*1024});
  await writeFile(resolve(work,"open.stdout.log"),child.stdout ?? "");
  await writeFile(resolve(work,"open.stderr.log"),child.stderr ?? "");
  await writeFile(resolve(work,"command.json"),JSON.stringify({command:["open",...openArgs],exitCode:child.status,signal:child.signal,error:String(child.error ?? ""),note:"open exits before the host; inspect host logs/result.json separately"},null,2));
  assert.equal(child.status,0,child.stderr || String(child.error));
  console.log(JSON.stringify({work,bundle,bundleId,openExit:child.status,note:"launch accepted; not evidence of runtime or input success"}));
}

