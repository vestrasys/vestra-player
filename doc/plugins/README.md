# Plugin documentation

Everything a plugin author needs is on the SDK's documentation site, [vestrasys.github.io/vestra-player-sdk](https://vestrasys.github.io/vestra-player-sdk/), and only there. This repository does not repeat it; link to the site instead.

| To | Read |
| --- | --- |
| Write a first plugin, and understand how plugins run | [Tutorial](https://vestrasys.github.io/vestra-player-sdk/documents/Tutorial.html) |
| Do one task (translate, test, publish, ...) | [Guides](https://vestrasys.github.io/vestra-player-sdk/documents/Guides.html) |
| Follow the interface and menu rules | [Design rules](https://vestrasys.github.io/vestra-player-sdk/documents/Design_rules.html) |
| Read complete example plugins | [Examples](https://vestrasys.github.io/vestra-player-sdk/documents/Examples.html) |
| Look up capabilities, limits, menu keys, bindings, platforms, theme, the CLI | [Reference](https://vestrasys.github.io/vestra-player-sdk/documents/Reference.html) |
| Look up one interface or member | [API](https://vestrasys.github.io/vestra-player-sdk/modules/_vestra-player_sdk.html) |

In this repository:

- `src/plugins/`: the built-in plugins (ids `vestra.<name>`, a prefix only they may use). `video/` is the shortest.
- How the player implements the plugin API: [doc/architecture/](../architecture/README.md); why the SDK is designed as it is: [sdk-design.md](../architecture/sdk-design.md).

`src/tests/sdk/host-agreement.test.ts` checks that the SDK's constants (the source of the site's Reference tables) agree with this player; `src/scripts/check-docs.mjs` keeps author documentation out of `doc/`.
