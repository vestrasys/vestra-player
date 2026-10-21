// Shared test setup: the real workshop (real engine) on the SDK test host, with
// scripted `packages.*` answers. Nothing is written inside the repository.
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { after } from "node:test";
import { TestHost, startPlugin } from "@vestra-player/sdk/testing";
import activate from "../main.ts";

const manifest = { id: "community.counter", version: "1.0.0", apiVersion: "vestra/1", entry: "main.js", capabilities: ["ui.window"] };

export const defaultPackages = () => ({
  validate: () => ({ token: "validated", digest: "digest", apiVersion: manifest.apiVersion, manifest, checks: ["slint"], trustedExecution: true }),
  installed: () => ({ revision: 7, current: null, previous: null }),
  install: ({ requestId }) => ({ operationId: requestId, status: "committed" }),
  rollback: ({ requestId }) => ({ operationId: requestId, status: "committed" }),
  operation: ({ operationId }) => ({ operationId, status: "committed" }),
  trial: () => ({ trialId: 1, status: "running" }),
  trialState: ({ trialId }) => ({ trialId, status: "running" }),
  stopTrial: ({ trialId }) => ({ trialId, status: "stopped", cleanupConfirmed: true }),
  cancel: ({ operationId }) => ({ operationId, status: "cancelled" }),
});

const workspaces = [];
// Also after a failed test: no candidate directories are left behind.
after(() => Promise.all(workspaces.map((dir) => fs.rm(dir, { recursive: true, force: true }))));

export async function startWorkshop(overrides = {}) {
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "vestra-workshop-test-"));
  workspaces.push(workspace);
  const host = new TestHost();
  for (const [name, handler] of Object.entries({ ...defaultPackages(), ...overrides })) host.handle(`packages.${name}`, handler);
  const { stop } = await startPlugin(activate, { directory: new URL("..", import.meta.url), host });
  await host.command("workshop.open").run();
  const window = host.window("workshop");
  const props = window.props;
  /** Fire a window callback and let the SDK finish the resulting requests. */
  const send = async (callback, ...args) => {
    window.emit(callback, ...args);
    await host.settle();
  };
  /** Wait for a condition that depends on local work (Pi, tsc) or SDK polling. */
  const until = async (condition, what = "condition") => {
    for (const deadline = Date.now() + 30_000; !condition(); ) {
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}; status: ${props["status-text"]}`);
      await new Promise((resolve) => setTimeout(resolve, 20)); // not settle(): a held reply stays in flight
    }
  };
  const status = () => String(props["status-text"]);
  const chooseWorkspace = async () => {
    host.dialogAnswers.push([workspace]);
    await send("choose-workspace");
  };
  /** Choose the workspace and run the real generation up to host validation. */
  const generate = async () => {
    await chooseWorkspace();
    await send("generate");
    await until(() => /候选已通过宿主校验|错误/.test(status()) && props.busy === false, "generation");
  };
  return {
    host,
    window,
    props,
    workspace,
    send,
    until,
    status,
    chooseWorkspace,
    generate,
    async stop() {
      await stop();
      await fs.rm(workspace, { recursive: true, force: true });
    },
  };
}
