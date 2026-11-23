//! Test plugins written when a manifest declared capabilities (before
//! player 0.9.0, SDK 1.9.0). The tests still say which ones they meant,
//! and this makes what the declarations switched on, as the SDK's own tests
//! do (its `test/explicit.ts`): `playback.read` → `await
//! plugin.player.read()`, first in `activate` (a wrapper around the test's
//! own). `events` (until SDK 1.11 the call `plugin.player.subscribe()`)
//! means nothing any more: an event reaches a plugin from its first
//! listener. The rest meant nothing at run time and is dropped: the
//! manifests the tests write have no `capabilities`.

/// `main` (a module with one `export default` activation) with the calls
/// `capabilities` stand for; unchanged when they stand for none.
pub fn main(capabilities: &[&str], main: &str) -> String {
    let read = capabilities.contains(&"playback.read");
    if !read {
        return main.to_owned();
    }
    assert_eq!(
        main.matches("export default ").count(),
        1,
        "one default export:\n{main}"
    );
    let own = main.replacen("export default ", "const declaredActivate = ", 1);
    format!(
        "{own}\nexport default async function activate(plugin) {{\n  await plugin.player.read();\n  return declaredActivate(plugin);\n}}\n"
    )
}
