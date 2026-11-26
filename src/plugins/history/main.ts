import type { Plugin, PlayerState, StateStream } from "@vestra-player/sdk";
import { addResumeCommands, quiet, RecentMenu } from "./menu.ts";
import { Recent } from "./recent.ts";
import { Resume } from "./resume.ts";

/**
 * Playback History: Open ▸ Recently Played (A12) and Resume Playback (B25, command palette only). Sources are
 * seen only through `player.state`, whoever opened them; reopening uses
 * `player.open`. Data lives in `plugin.storage` files (recent.json,
 * positions.json), written after each change and on deactivation.
 *
 * The state is followed with `player.stream` while a media source is open
 * (M5), so the playing position is saved every few
 * seconds and little is lost at a quit or a crash. The "open" trigger opens the
 * stream when a source opens (and at a start while one is open: its `enter` is
 * delivered after the plugin is ready, so the stream is not something held at
 * the ready) and stops it when the last source closes; with nothing open the
 * plugin holds nothing and may be stopped while idle, and the next source wakes
 * it.
 *
 * Known limit (accepted for this release): a start woken by a source parks again
 * as described, but on a first run, a restart after a crash or a launch with a
 * file already open the plugin stays resident for that run. An SDK "after
 * ready" hook is recorded as a follow-up.
 */
export default async function activate(plugin: Plugin) {
  const { player, when } = plugin;
  let feed: StateStream | null = null;
  /** The open stream has sent its first state. */
  let streamed = false;
  const unfollow = () => {
    feed?.stop();
    feed = null;
    streamed = false;
  };
  let observe = (_state: PlayerState) => {};
  const follow = () => {
    if (feed) return;
    feed = player.stream((state) => {
      streamed = true;
      observe(state);
    });
  };
  // Given again at every start (after `off`), so a start while a source is open
  // enters it too: a kept "entered" would send no `enter` (a reload, an upgrade).
  plugin
    .trigger("open")
    .enter(follow)
    .leave(unfollow)
    .off()
    .firesOn(when.open.and(when.stream.not()));

  await plugin.player.read();
  const recent = new Recent(plugin);
  const resume = new Resume(plugin, plugin.osd);
  const menu = new RecentMenu(plugin, recent, resume, plugin.osd);
  addResumeCommands(plugin, resume, plugin.osd);
  // The player's own message, shown as it is.
  // i18n-ignore
  plugin.onError((error) => quiet(plugin.osd.show(error.message)));

  observe = (state: PlayerState) => {
    recent.observe(state);
    resume.observe(state);
  };
  // The source events' states until the stream sends its own: a source that
  // opened and became ready before the stream's first state (several opened in quick
  // succession, or the ones that woke the plugin) is still seen.
  player.on("source.opening", ({ state }) => {
    // Opened after all: a start for this source was woken by it, not a reload.
    resume.opened(state.sourceId);
    if (!streamed) observe(state);
  });
  player.on("source.ready", ({ state }) => {
    if (!streamed) observe(state);
  });

  recent.observe(player.state);
  resume.observe(player.state, true);
  void menu.build();
}
