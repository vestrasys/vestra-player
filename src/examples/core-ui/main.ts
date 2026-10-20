import type { Plugin } from "../../sdk/plugin/index.ts";

/**
 * Demonstrates the core UI interfaces (doc/architecture/core-ui-design.md):
 * menu path/order/group/radio, bindings with repeat, a command outside the
 * menu, the on-screen display, next/previous, the shared sidebar state with
 * detach/attach, and a tool window built from the theme and widgets.
 */
export default async function activate(plugin: Plugin) {
  const { player, osd } = plugin;
  const sidebar = plugin.panels.sidebar;
  const menu = (title: string) => ["示例", title];
  const say = (text: string) => osd.show(text);

  // Menu: one top-level submenu in its own separator group, radio items.
  let speed = 1;
  for (const [index, rate] of [0.5, 1, 2].entries())
    plugin.commands.add({
      id: `rate-${rate}`,
      path: ["示例", "速度", `${rate}倍`],
      order: 850 + index,
      group: "examples",
      radio: true,
      checked: () => speed === rate,
      enabled: () => player.state.can.rate,
      run: async () => {
        await player.setRate(rate);
        speed = rate;
        await say(`播放速度: ${rate}x`);
      },
    });

  // Bindings: a held key repeats; a command with no menu entry.
  plugin.commands.add({
    id: "volume-up",
    path: menu("音量 +5"),
    order: 860,
    group: "examples",
    bindings: ["Ctrl+Alt+ArrowUp"],
    repeat: true,
    run: async () => {
      const volume = Math.min(200, player.state.volume + 5);
      await player.setOutput({ volume });
      await say(`音量: ${volume}%`);
    },
  });
  plugin.commands.add({
    id: "where",
    title: "显示当前位置",
    bindings: ["Ctrl+Alt+W"],
    run: () => say(`位置: ${Math.round(player.state.position ?? 0)} 秒`),
  });

  // Next / previous through whichever plugin provides the sequence.
  for (const [id, title, key, step] of [
    ["next", "下一个", "Ctrl+Alt+N", () => player.next()],
    ["previous", "上一个", "Ctrl+Alt+P", () => player.previous()],
  ] as const)
    plugin.commands.add({
      id,
      path: menu(title),
      order: id === "next" ? 861 : 862,
      group: "examples",
      bindings: [key],
      enabled: () => player.state.can[id],
      run: step,
    });

  // The sidebar, whoever mounted it: show/hide and float/attach (C7, C14).
  plugin.commands.add({
    id: "sidebar",
    path: menu("显示侧栏"),
    order: 863,
    group: "examples",
    checked: () => sidebar.state?.preferred ?? false,
    enabled: () => sidebar.state !== null,
    run: () => sidebar.toggle(),
  });
  plugin.commands.add({
    id: "detach",
    path: menu("分离侧栏"),
    order: 864,
    group: "examples",
    bindings: ["Ctrl+Alt+D"],
    checked: () => sidebar.state?.detached ?? false,
    enabled: () => sidebar.state !== null,
    run: () => sidebar.setDetached(!sidebar.state?.detached),
  });

  // A tool window drawn with @vestra/theme.slint and @vestra/widgets.slint.
  const window = plugin.windows.create("theme.slint");
  let level = 0.5;
  window.bind(() => ({ level, label: `亮度 ${Math.round(level * 100)}` }));
  window.on("levelChanged", (value: number) => (level = value));
  window.on("reset", () => (level = 0.5));
  plugin.commands.add({
    id: "theme",
    path: menu("主题控件…"),
    order: 865,
    group: "examples",
    run: () => window.show(),
  });
}
