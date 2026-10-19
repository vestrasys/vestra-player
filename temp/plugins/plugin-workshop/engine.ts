// 实验工坊，目标契约 probe-0.2；最终插件 API 与本实现可能差异很大。
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  Agent,
  type AgentTool,
  type StreamFn,
} from "@earendil-works/pi-agent-core";
import { Type } from "typebox";
import type { Api, Model } from "@earendil-works/pi-ai";
import type { PluginContext, UiProperties } from "../../sdk/types.ts";
import {
  createMockStream,
  generatedId,
  mockModel,
  type Request,
} from "./model.ts";

const writeSchema = Type.Object(
  {
    files: Type.Array(
      Type.Object(
        {
          name: Type.String(),
          contents: Type.String(),
        },
        { additionalProperties: false },
      ),
      { minItems: 4, maxItems: 4 },
    ),
  },
  { additionalProperties: false },
);
const emptySchema = Type.Object({}, { additionalProperties: false });
const packageFiles = new Set([
  "index.ts",
  "ui.slint",
  "manifest.json",
  "package.json",
]);

async function removePackageSources(manifest: string) {
  await Promise.all(
    [...packageFiles].map((name) =>
      rm(join(dirname(manifest), name), { force: true }),
    ),
  );
}

export interface RunEvidence {
  request: Request;
  baseline: "probe-0.2";
  pi: "@earendil-works/pi-agent-core@1.3.0";
  provider: "local-mock";
  outcome: "installed" | "failed" | "cancelled";
  validations: { valid: boolean; diagnostics: string[] }[];
  events: { type: string; tool?: string }[];
  transcript: unknown[];
  error?: string;
}
export interface ModelAdapter {
  model: Model<Api>;
  streamFn: StreamFn;
}
export type ModelFactory = (
  request: Request,
  sdkImport: string,
) => ModelAdapter;
const mockFactory: ModelFactory = (request, sdkImport) => ({
  model: mockModel,
  streamFn: createMockStream(request, sdkImport),
});

/** Package lifecycle orchestration stays in this optional plugin, not in the Rust core. */
export class Workshop {
  private active?: { agent: Agent; promise: Promise<void> };
  private disposed = false;
  private committing = false;
  private current?: string;
  private previous?: string;
  private serial = 0;
  private readonly ctx: PluginContext;
  readonly workspace: string;
  private readonly modelFactory: ModelFactory;
  private constructor(
    ctx: PluginContext,
    workspace: string,
    modelFactory: ModelFactory,
  ) {
    this.ctx = ctx;
    this.workspace = workspace;
    this.modelFactory = modelFactory;
  }
  static async create(
    ctx: PluginContext,
    modelFactory: ModelFactory = mockFactory,
  ) {
    const cache = fileURLToPath(new URL(".cache/", import.meta.url));
    await mkdir(cache, { recursive: true });
    const workspace = await mkdtemp(join(cache, "workshop-"));
    return new Workshop(ctx, workspace, modelFactory);
  }
  private async patch(properties: UiProperties) {
    if (!this.disposed) await this.ctx.ui.patch(properties);
  }
  get busy() {
    return this.active !== undefined;
  }
  async start(request: Request): Promise<void> {
    if (this.disposed) throw new Error("工坊已停用");
    if (this.active) throw new Error("已有生成任务；请取消或等待结束");
    const directory = join(this.workspace, `candidate-${++this.serial}`);
    await mkdir(directory);
    const manifest = join(directory, "manifest.json");
    const sdkImport = relative(
      directory,
      fileURLToPath(new URL("../../sdk/types.ts", import.meta.url)),
    ).replaceAll("\\", "/");
    const evidence: RunEvidence = {
      request,
      baseline: "probe-0.2",
      pi: "@earendil-works/pi-agent-core@1.3.0",
      provider: "local-mock",
      outcome: "failed",
      validations: [],
      events: [],
      transcript: [],
    };
    let revision = 0;
    let validatedRevision = -1;
    let installed = false;
    const writeTool: AgentTool<typeof writeSchema, { written: number }> = {
      name: "write_package",
      label: "写候选包",
      description:
        "写入独立候选目录内的 TS、Slint、manifest 和 package.json，不触碰已安装包。",
      parameters: writeSchema,
      async execute(_id, params, signal) {
        signal?.throwIfAborted();
        const names = new Set(params.files.map((file) => file.name));
        if (
          names.size !== 4 ||
          [...names].some((name) => !packageFiles.has(name))
        )
          throw new Error("只允许四个声明的候选文件");
        if (params.files.some((file) => file.contents.length > 65536))
          throw new Error("候选文件超过实验大小限制");
        for (const file of params.files)
          await writeFile(join(directory, file.name), file.contents, {
            signal,
          });
        revision += 1;
        validatedRevision = -1;
        return {
          content: [
            {
              type: "text",
              text: JSON.stringify({ written: params.files.length }),
            },
          ],
          details: { written: params.files.length },
        };
      },
    };
    const validateTool: AgentTool<
      typeof emptySchema,
      { valid: boolean; diagnostics: string[] }
    > = {
      name: "validate_package",
      label: "真实编译校验",
      description:
        "通过公共插件 API 运行真实 manifest、严格 TypeScript 和 Slint 校验，返回真实诊断。",
      parameters: emptySchema,
      execute: async (_id, _params, signal) => {
        signal?.throwIfAborted();
        await this.patch({
          phase: "validating",
          status: "宿主正在真实校验候选包",
        });
        const result = await this.ctx.plugins.validate(manifest);
        signal?.throwIfAborted();
        evidence.validations.push(result);
        validatedRevision = result.valid ? revision : -1;
        await this.patch({
          diagnostics: result.diagnostics.join("\n").slice(0, 1600),
        });
        return {
          content: [{ type: "text", text: JSON.stringify(result) }],
          details: result,
          isError: !result.valid,
        };
      },
    };
    const installTool: AgentTool<typeof emptySchema, { id: string }> = {
      name: "install_package",
      label: "试载并安装",
      description:
        "只安装本轮校验通过且未改动的候选；宿主在 ready 后原子替换旧 worker，失败保留旧版本。",
      parameters: emptySchema,
      execute: async (_id, _params, signal) => {
        signal?.throwIfAborted();
        if (validatedRevision < 1 || validatedRevision !== revision)
          throw new Error("当前候选未通过校验");
        // 公共 install 是事务提交，进入后不谎称可以中途取消。失败由宿主保留旧 worker。
        this.committing = true;
        await this.patch({
          phase: "installing",
          status: "试载候选；成功 ready 后安装",
        });
        try {
          const result = await this.ctx.plugins.install(manifest);
          const obsolete = this.previous;
          this.previous = this.current;
          this.current = manifest;
          if (obsolete && obsolete !== this.previous) {
            await removePackageSources(obsolete).catch((error) => {
              console.error("清理旧候选源文件失败：", error);
            });
          }
          installed = true;
          return {
            content: [{ type: "text", text: JSON.stringify(result) }],
            details: result,
          };
        } finally {
          this.committing = false;
        }
      },
    };
    const modelAdapter = this.modelFactory(
      request,
      sdkImport.startsWith(".") ? sdkImport : `./${sdkImport}`,
    );
    const agent = new Agent({
      initialState: {
        model: modelAdapter.model,
        systemPrompt:
          "你是实验插件作者。生成独立的计数器 TS/Slint/manifest 包。先写候选，再校验；根据真实诊断修复。只安装已通过校验的包。禁止访问其他插件或宿主内部。",
        tools: [writeTool, validateTool, installTool],
      },
      streamFn: modelAdapter.streamFn,
      toolExecution: "sequential",
    });
    let turns = 0;
    const unsubscribe = agent.subscribe(async (event) => {
      if (event.type === "turn_start") {
        turns += 1;
        await this.patch({ turns });
      }
      evidence.events.push({
        type: event.type,
        ...(event.type.startsWith("tool_execution") && "toolName" in event
          ? { tool: event.toolName }
          : {}),
      });
    });
    await this.patch({
      busy: true,
      phase: "generating",
      status: "Pi Agent 正在调用本地 mock 模型",
      request: request.prompt,
      diagnostics: "",
      turns: 0,
      report: "",
    });
    // 不阻塞宿主 action 队列；取消操作仍能抵达同一 worker。
    const promise = (async () => {
      try {
        await agent.prompt(
          `${request.prompt}\n计数增量：${request.increment}。本轮使用本地 mock 模型，不调用网络。`,
        );
        evidence.transcript = agent.state.messages;
        evidence.outcome = installed
          ? "installed"
          : agent.signal?.aborted
            ? "cancelled"
            : "failed";
        // Agent 结束后 active signal 可能被清空，所以也检查最后的实际 assistant 结果。
        const last = agent.state.messages.at(-1);
        if (
          !installed &&
          last?.role === "assistant" &&
          last.stopReason === "aborted"
        )
          evidence.outcome = "cancelled";
        if (agent.state.errorMessage) evidence.error = agent.state.errorMessage;
        if (!installed && !evidence.error) {
          const failedTool = [...agent.state.messages]
            .reverse()
            .find(
              (message) => message.role === "toolResult" && message.isError,
            );
          if (failedTool?.role === "toolResult")
            evidence.error = failedTool.content
              .filter((content) => content.type === "text")
              .map((content) => content.text)
              .join("\n");
        }
      } catch (error) {
        evidence.error = String(error);
        evidence.transcript = agent.state.messages;
      } finally {
        unsubscribe();
        const report = join(directory, "run.json");
        try {
          await writeFile(report, JSON.stringify(evidence, null, 2));
          if (!installed) await removePackageSources(manifest);
          await this.patch({
            report,
            busy: false,
            phase: evidence.outcome,
            installed: this.current ? generatedId : "",
            status:
              evidence.outcome === "installed"
                ? "生成、诊断修复、校验与安装完成"
                : evidence.outcome === "cancelled"
                  ? "已取消；已安装版本不变"
                  : "候选失败；已安装版本不变",
            diagnostics:
              evidence.error ??
              evidence.validations
                .filter((v) => !v.valid)
                .flatMap((v) => v.diagnostics)
                .join("\n")
                .slice(0, 1600),
          });
        } finally {
          this.active = undefined;
        }
      }
    })();
    this.active = { agent, promise };
    // 后台异常必须可观察，避免 worker unhandled rejection。
    void promise.catch((error) => {
      console.error("工坊记录失败：", error);
    });
  }
  async cancel() {
    if (!this.active) return;
    if (this.committing) {
      await this.patch({
        status: "安装已进入事务提交；等待成功或失败后再回退",
      });
      return;
    }
    this.active.agent.abort();
    await this.active.promise;
  }
  async rollback() {
    if (this.active) throw new Error("生成期间不能回退");
    if (!this.previous) throw new Error("没有可回退版本");
    const previous = this.previous;
    await this.ctx.plugins.install(previous);
    this.previous = this.current;
    this.current = previous;
    await this.patch({
      phase: "rolled-back",
      installed: generatedId,
      status: "已重新试载并回退上一个生成包；计数器重新初始化",
    });
  }
  async remove() {
    if (this.active) throw new Error("生成期间不能移除");
    if (!this.current) return;
    await this.ctx.plugins.remove(generatedId);
    await removePackageSources(this.current);
    if (this.previous) await removePackageSources(this.previous);
    this.current = undefined;
    this.previous = undefined;
    await this.patch({
      phase: "removed",
      installed: "",
      status: "生成插件已卸载；证据仍在工作区",
    });
  }
  async dispose() {
    const active = this.active;
    await this.cancel();
    if (active) await active.promise;
    this.disposed = true;
    // 已安装包是独立插件，不能随工坊 UI 卸载而删源文件；保留安装包和证据供宿主管理。
  }
}

export function parseRequest(value: unknown): Request {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("需求必须是 JSON 对象");
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).some((key) => key !== "prompt" && key !== "increment")
  )
    throw new Error("不支持的需求字段");
  if (
    typeof record.prompt !== "string" ||
    record.prompt.trim().length < 1 ||
    record.prompt.length > 2000
  )
    throw new Error("prompt 必须是 1–2000 字符");
  if (
    typeof record.increment !== "number" ||
    !Number.isInteger(record.increment) ||
    record.increment < 1 ||
    record.increment > 10
  )
    throw new Error("increment 必须是 1–10 整数");
  return {
    prompt: record.prompt,
    increment: record.increment,
    scenario: "repair",
  };
}
