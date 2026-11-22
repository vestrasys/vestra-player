import type { Plugin, PlayerState } from "@vestra-player/sdk";
import { Actions } from "./actions.ts";
import { Bookmarks } from "./bookmarks.ts";
import { Chapters, chapterRanges, currentChapter, hasMedia } from "./chapters.ts";
import { ListWindow } from "./list.ts";
import { MarksMenu, MENU_ITEMS } from "./menu.ts";
import { osdOf } from "./osd.ts";
import { addSeekbarCommand, SeekbarMarks } from "./seekbar.ts";
import { Settings } from "./store.ts";

/**
 * Chapters and Bookmarks: Chapter and Bookmark List… in Tools ▸ and the other
 * Chapters and Bookmarks commands in the command palette (B21: chapters from
 * `player.chapters()`, bookmarks per file in bookmarks.json, the Chapters and
 * Bookmarks window) and the seek-bar marks (B22: this plugin's `timeline`
 * layer, the switch in marks.json). A-B Loop (B23) moved to the playback
 * plugin's Playback panel.
 *
 * It follows the source, whoever opened it, through the source events
 * (`source.opening`, `source.ready`; at `source.ended` the state is read again,
 * so a close is seen), and holds no stream unless the window shows: the
 * commands' state is `plugin.when` conditions, the reason follows the "media"
 * trigger, and the current chapter (the radio check) the "chapter" trigger's
 * range list. With nothing shown the plugin may be stopped while idle; its
 * seek-bar layer stays with the player.
 */
export default async function activate(plugin: Plugin) {
  const { player, when } = plugin;
  /** Facts the triggers keep: a media source is open (`hasMedia`), the chapter playing (-1: none). */
  const facts = plugin.state({ media: false, chapter: -1 });
  plugin
    .trigger("media")
    .enter(() => void (facts.media = true))
    .leave(() => void (facts.media = false))
    .firesOn(when.phase("ready", "ended").and(when.stream.not()));
  /** The chapter of each range of the "chapter" trigger's list. */
  let rangeChapters: number[] = [];
  const chapterTrigger = plugin
    .trigger("chapter")
    .enter(({ range }) => void (facts.chapter = range ? (rangeChapters[range.index] ?? -1) : -1))
    .leave(() => void (facts.chapter = -1));

  await player.read();
  facts.media = hasMedia(player.state);
  const osd = osdOf(plugin);
  plugin.onError((error) => {
    // i18n-ignore: the player's (or the SDK's) own message
    void plugin.osd.show(error.message).catch(() => {});
  });

  const settings = new Settings(plugin);
  const chapters = new Chapters(player, () => plugin.update());
  const bookmarks = new Bookmarks(plugin);
  const actions = new Actions(plugin, chapters, bookmarks, osd, facts);
  const list = new ListWindow(plugin, chapters, bookmarks, actions);
  const menu = new MarksMenu(plugin, chapters, bookmarks, actions, () => list.show(), facts);
  const seekbar = new SeekbarMarks(plugin, settings, chapters, bookmarks, osd);
  addSeekbarCommand(plugin, seekbar);

  // The current chapter: the chapters the menu lists, as one range list bound to
  // their source. Taken off first, so the player enters the range playing now
  // even when it kept the same list as entered (a restart for the same source).
  chapters.onChange(() => {
    const { ranges, chapters: of } = chapterRanges(chapters.list, MENU_ITEMS);
    rangeChapters = of;
    // Until the player says otherwise: the chapter at the position last seen.
    const playing = currentChapter(chapters.list, player.state.position);
    facts.chapter = playing < MENU_ITEMS ? playing : -1;
    chapterTrigger.off();
    if (ranges.length) chapterTrigger.firesOn(when.position.in(ranges));
  });

  const observe = (state: PlayerState) => {
    chapters.observe(state);
    bookmarks.open(hasMedia(state) || state.phase === "loading" ? state.source : null, state.sourceId);
    seekbar.schedule();
  };
  player.on("source.opening", ({ state }) => observe(state));
  player.on("source.ready", ({ state }) => observe(state));
  // The data is that source's last state: read the state now (empty after a close,
  // `ended` at the end of the file, the next source after a replacement).
  player.on("source.ended", () =>
    player.refresh().then(observe, () => {
      // Stopping (the player quits): nothing to follow.
    }),
  );
  observe(player.state);
  void menu.build();
}
