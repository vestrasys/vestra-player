// 实验：只替换模型调用，Pi Agent 的工具编排、诊断反馈和取消保持真实。
import type { StreamFn } from "@earendil-works/pi-agent-core";
import {
  createAssistantMessageEventStream,
  type AssistantMessage,
  type Model,
  type ToolCall,
  type ToolResultMessage,
} from "@earendil-works/pi-ai";

export type Scenario =
  | "repair"
  | "invalid"
  | "activation-failure"
  | "slow"
  | "slint-failure";
export interface Request {
  prompt: string;
  increment: number;
  scenario: Scenario;
}
export const generatedId = "workshop-generated-counter";
export const mockModel: Model<"openai-completions"> = {
  id: "deterministic-workshop-replay",
  name: "本地 mock（零网络）",
  api: "openai-completions",
  provider: "vestra-mock",
  baseUrl: "http://invalid.localhost",
  reasoning: false,
  input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  contextWindow: 32000,
  maxTokens: 8000,
};

// 这是模型响应的可重放素材，不是从模板目录复制插件。源码经 write_package 工具执行。
function packageResponse(
  request: Request,
  sdkImport: string,
  repaired: boolean,
): ToolCall["arguments"] {
  const broken =
    !repaired &&
    request.scenario !== "activation-failure" &&
    request.scenario !== "slint-failure";
  const source = `// 实验生成插件；最终 SDK 可能差异很大。\nimport type { PluginContext } from ${JSON.stringify(sdkImport)};\nexport async function activate(ctx: PluginContext) {\n  const increment: number = ${broken ? '"not-a-number"' : request.increment};\n  ${request.scenario === "activation-failure" ? 'throw new Error("intentional candidate activation failure");' : ""}\n  let count = 0;\n  await ctx.ui.patch({ count, increment, status: "generated-ready" });\n  const unsubscribe = ctx.onAction("increment", async () => {\n    count += increment;\n    await ctx.ui.patch({ count, increment, status: "generated-ready" });\n  });\n  return () => { unsubscribe(); };\n}\n`;
  const ui = `// 实验生成原生 Slint；不是最终 API。\nimport { Button } from "std-widgets.slint";\nexport global GeneratedCounterState {\n  in-out property <float> count: 0;\n  in-out property <float> increment: 1;\n  in-out property <string> status: "starting";\n  callback action(string, float);\n}\nexport component GeneratedCounterPanel inherits Rectangle {\n  background: #233d36;\n  HorizontalLayout {\n    padding: 10px;\n    Text { text: "生成计数器：" + GeneratedCounterState.count; color: white; }\n    Button { text: "+" + GeneratedCounterState.increment; clicked => { GeneratedCounterState.action("increment", 0); } }\n  }\n}\n${request.scenario === "slint-failure" ? "this is not valid slint;" : ""}\n`;
  const manifest = {
    id: generatedId,
    name: `工坊计数器 +${request.increment}（生成实验）`,
    experimental: true,
    entry: "index.ts",
    ui: {
      file: "ui.slint",
      component: "GeneratedCounterPanel",
      global: "GeneratedCounterState",
    },
    commands: [
      {
        id: "increment",
        title: `计数器增加 ${request.increment}`,
        menu: ["工具", "生成插件"],
      },
    ],
  };
  return {
    files: [
      { name: "index.ts", contents: source },
      { name: "ui.slint", contents: ui },
      { name: "manifest.json", contents: JSON.stringify(manifest, null, 2) },
      {
        name: "package.json",
        contents: JSON.stringify({ type: "module", private: true }),
      },
    ],
  };
}
function textOf(result: ToolResultMessage): string {
  return result.content
    .filter((c) => c.type === "text")
    .map((c) => c.text)
    .join("\n");
}

/** A replaceable Pi StreamFn. Real providers can use Models.streamSimple at this same seam. */
export function createMockStream(
  request: Request,
  sdkImport: string,
): StreamFn {
  let calls = 0;
  return (_model, context, options) => {
    const stream = createAssistantMessageEventStream();
    const message: AssistantMessage = {
      role: "assistant",
      content: [],
      api: mockModel.api,
      provider: mockModel.provider,
      model: mockModel.id,
      usage: {
        input: 0,
        output: 0,
        cacheRead: 0,
        cacheWrite: 0,
        totalTokens: 0,
        cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
      },
      stopReason: "stop",
      timestamp: Date.now(),
    };
    const emit = () => {
      if (options?.signal?.aborted) {
        message.stopReason = "aborted";
        message.errorMessage = "mock model cancelled";
        stream.push({ type: "error", reason: "aborted", error: message });
        return;
      }
      const results = context.messages.filter(
        (m): m is ToolResultMessage => m.role === "toolResult",
      );
      const last = results.at(-1);
      let tool: { name: string; arguments: ToolCall["arguments"] } | undefined;
      if (++calls > 12) {
        message.stopReason = "error";
        message.errorMessage = "mock exceeded bounded turn limit";
        stream.push({ type: "error", reason: "error", error: message });
        return;
      }
      if (!last)
        tool = {
          name: "write_package",
          arguments: packageResponse(request, sdkImport, false),
        };
      else if (last.toolName === "write_package" && !last.isError)
        tool = { name: "validate_package", arguments: {} };
      else if (last.toolName === "validate_package") {
        let validation: unknown;
        try {
          validation = JSON.parse(textOf(last));
        } catch {
          validation = undefined;
        }
        const record =
          validation && typeof validation === "object"
            ? (validation as Record<string, unknown>)
            : {};
        if (record.valid === true)
          tool = { name: "install_package", arguments: {} };
        else if (
          request.scenario !== "invalid" &&
          request.scenario !== "slint-failure" &&
          Array.isArray(record.diagnostics) &&
          record.diagnostics.some(
            (d) => typeof d === "string" && d.includes("TS2322"),
          ) &&
          results.filter((r) => r.toolName === "validate_package").length === 1
        ) {
          // 只有实际类型诊断进入 Pi 的下一次模型调用，才产生修复；没有预定假成功结果。
          tool = {
            name: "write_package",
            arguments: packageResponse(request, sdkImport, true),
          };
        }
      }
      if (tool) {
        message.content = [{ type: "toolCall", id: `mock-${calls}`, ...tool }];
        message.stopReason = "toolUse";
      } else
        message.content = [
          {
            type: "text",
            text: last?.isError
              ? "候选失败，保留已安装版本。"
              : "本轮工具调用结束。",
          },
        ];
      stream.push({ type: "start", partial: message });
      stream.push({ type: "done", reason: tool ? "toolUse" : "stop", message });
    };
    if (request.scenario !== "slow") queueMicrotask(emit);
    else {
      const timer = setTimeout(() => {
        options?.signal?.removeEventListener("abort", abort);
        emit();
      }, 2000);
      function abort() {
        clearTimeout(timer);
        emit();
      }
      options?.signal?.addEventListener("abort", abort, { once: true });
      if (options?.signal?.aborted) abort();
    }
    return stream;
  };
}
