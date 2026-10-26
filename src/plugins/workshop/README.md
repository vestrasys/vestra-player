# Vestra Player AI (experimental)

An AI assistant inside the player, based on [pi](https://github.com/earendil-works/pi). Use your own API key and chat with the model you choose; the AI can write Vestra plugins in the current chat's folder, type-check, test and pack them itself, and you can install the finished plugin or export it.

## How to use it

1. Choose "Vestra Player AI…" in the right-click menu.
2. The first time, click "Go to Settings" (or "⚙ Settings" in the sidebar), and in "Options ▸ Plugins ▸ Vestra Player AI":
   - choose a provider: all the providers pi supports that use an API key (Anthropic, OpenAI, Google, OpenRouter, DeepSeek, Mistral, Groq, xAI, MiniMax, Moonshot and others, pay-as-you-go), or "Custom · OpenAI format" / "Custom · Anthropic format" (enter a Base URL; works with Ollama, LM Studio or other compatible services);
   - paste the API key and click "Save"; each provider's key is saved separately;
   - choose a model. For a custom service, press Enter or click "Fetch Models" after entering the Base URL to ask the service for its models; if it cannot list them, enter the model ID directly.
3. Back in the AI window, type a message; Enter sends, Shift+Enter starts a new line. While a reply is being generated you can click "■ Stop".
4. While the AI works there is one status line: it first shows "Thinking…", and while a tool runs it shows the current tool (each new one replaces the last); when the turn is over, click the line to expand all tool calls.
5. On the left is the chat list: "+ New Chat" starts a new chat; click an item to switch to it. When you close the window and open it again, you return to the last chat.
6. "Open Folder" opens the current chat's working folder.
7. Enter your GitHub user name under "Plugin author" on the settings page: plugins the AI creates get the id `<user name>.<name>` (the official plugin repository requires the id prefix to be the GitHub user or organization that owns the source repository). Without it the id is `local.<name>`, which is only for use on your own machine; change the id before publishing. `vestra.` is reserved for official plugins and cannot be used.

## Writing plugins

When you ask the AI to write a plugin, it first reads the plugin SDK reference (read-only, inside the plugin package) and then uses these tools; the status line shows their display names:

| Tool | Status line | What it does |
| --- | --- | --- |
| `scaffold_plugin` | Create plugin | Creates a project in the layout of the official plugin repository: manifest.json, main.ts, ui/, tests/, README, CHANGELOG |
| `typecheck` | Type check | Strict type check (with the SDK types that come with the player); errors include file, line and error code |
| `test` | Run tests | Runs `tests/*.test.ts` in a separate restricted Node process: it can only read the plugin folder and only write a temporary folder, with no network and no child processes; on failure it returns the assertion messages and what the tests printed |
| `check` | Check | id, dependencies, manifest, types, build (Slint is compiled by the player at install time) |
| `pack` | Pack | Creates `dist/<id>-<version>.vxp` |
| `install_plugin` | Install | The player validates it (including Slint), asks for confirmation in a dialog, then installs it (only when you ask) |

The AI is done only when type check, tests and check all pass. Every tool has a time limit and an output limit; clicking "■ Stop" cancels the running tool. When the `vestra-plugin` command line fails without its usual report, the tool result carries its exit code and the end of its error output, never an empty failure.

When the chat has a plugin project, a plugin card appears above the input box once the AI has finished its reply (while it is still working, the card stays hidden; it comes back with the latest state when the reply ends):

- "Install": installs it into the player (the source is recorded as the AI workshop); if a plugin with the same id came from elsewhere, you are asked first whether to replace it. Installing again after a change upgrades the plugin in place and keeps its data. To remove it, use Uninstall… in the Plugin Manager. If, after installing, the plugin's shortcuts are taken by the player or another plugin (the player ignores those keys), both the card and the AI are told; for this the plugin reads the shortcut table (`bindings.manage`, read only; it changes no keys).
- "Export .vxp": packs the plugin and saves a copy you can share.
- "Export Source .zip": the complete plugin project, ready to become your own GitHub repository: README, CHANGELOG, tests, tsconfig, a package.json with the SDK dependency, a GitHub Actions workflow (check, test, pack), and a LICENSE placeholder to replace; without node_modules and dist.
- "Open Folder": the plugin project's folder.

## Notes

- **How keys are stored**: only in the system keychain (macOS) or Credential Manager (Windows), never in any file, log or chat history. The cost of calling the model is charged to your provider account.
- **Reading the key**: it is read only when the AI window opens, and kept in memory after the first read. The system may show a permission dialog (common after a player update); if you deny it or reading fails, the top of the window says "Cannot read the key from the keychain; save it again in the settings", and everything else keeps working.
- **What the AI can do**: it can only read and write the current chat's working folder (`read`, `write`, `edit`, `ls`, `find`, `grep`, `delete`; `delete` only removes files or empty folders, for cleaning up debug files), and it can also read the plugin SDK reference (read-only, short paths `sdk/…`, for example `sdk/llms.txt`; models often copy long absolute paths wrongly). It cannot run commands, cannot access other files, and cannot use symbolic links or file names starting with ".". The AI runs in a separate restricted process. The plugin tools are run by the plugin process with fixed arguments. Tests and type checks (typecheck) run in Node's permission mode (`--permission`): they can only read the plugin project, the SDK and the SDK copy in the data directory, only write a temporary folder, and have no network; tests cannot start child processes; type checks can, but only to start `tsc`, which inherits the same limits. check and pack do not run in permission mode yet (the `fs.cp` the SDK command line uses to copy files reads every parent directory of the target in permission mode).
- **Where chats are stored**: in `sessions/<chat>/` in the plugin data directory: `meta.json`, pi's chat history (`.jsonl`), the working folder `work/` (one `work/<id>/` per plugin) and the pack results `dist/`. `node_modules/` in the data directory is a copy of the plugin SDK, used for type checks and tests.
- Only one chat generates at a time.
- The opening line before the first tool call of a reply ("I'll start by reading…") is not shown: models usually write it in English, and the tool status line already says what is happening. The chat history keeps the original text.
- AI replies are shown as Markdown (headings, lists, tables, code blocks and so on); code blocks have "Copy" in the top right corner, and links open in the browser. Web images in replies are not downloaded; only their alt text is shown. Very long chats show only the most recent messages; the full history is in the session folder.
- The provider list has only pay-as-you-go APIs; subscription plans such as coding plans or Token Plans are not in the list; to use one, connect it with "Custom · OpenAI format" or "Custom · Anthropic format".
- Needs Vestra Player 0.4.3 or later. The interface texts are written in English; the Simplified Chinese translation is in `locales/zh-Hans.po` (`vestra-plugin i18n extract` updates `locales/messages.pot`).
- The plugin SDK (`vendor/@vestra-player/sdk`, including the `vestra-plugin` command line), TypeScript and `@types/node` come with the plugin, at the same SDK version the player uses (`npm run vendor:sdk` updates them, `tests/vendor-sdk.mjs`). The reference the AI reads is the `llms.txt` and `llms-full.txt` shipped in the SDK package.
- Evaluation: `tests/eval/` has 6 fixed tasks; `npm run eval` runs them with a real model (this costs money on your provider account; run it by hand only after changing the prompt or the tools), see the notes at the top of `tests/eval/run.mjs`.
- Licenses of third-party components are in `licenses/` (shipped with the plugin package); the source of the pi logo is in `licenses/pi-logo/NOTICE.md`; the licenses of the SDK, TypeScript, `@types/node` and `undici-types` shipped with the plugin are in their own folders under `vendor/` (`LICENSE`, `LICENSE.txt`, `ThirdPartyNoticeText.txt`).
