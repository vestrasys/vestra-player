import type { Plugin, TemplateArgs } from "@vestra-player/sdk";

/**
 * Every OSD template this plugin shows. The player translates the template
 * and fills in `args` (times, titles, counts: data); `vestra-plugin i18n
 * extract` reads the texts from this type.
 */
export type OsdText =
  // B21 chapters and bookmarks
  | "Chapter: {}"
  | "Chapter: Chapter {}"
  | "Bookmark: {}"
  | "Bookmark added: {} ({})"
  | "A bookmark already exists here"
  | "Bookmarks full ({})"
  | "Can't add bookmarks to this source"
  | "No previous chapter"
  | "No next chapter"
  | "No previous bookmark or chapter"
  | "No next bookmark or chapter"
  | "Can't seek: {}"
  // B22 seek bar
  | "Bookmark and chapter marks on the seek bar: on"
  | "Bookmark and chapter marks on the seek bar: off"
  // B23 A-B loop
  | "A-B loop: {} ~ {}"
  | "A-B start: {}"
  | "A-B end: {}"
  | "A-B loop ended"
  | "A-B loop off"
  | "Start point is after the end point; end point cleared"
  | "End point must be after the start point"
  | "Start point cleared"
  | "End point cleared"
  | "Reached the beginning"
  | "Reached the end"
  | "Set the start and end points first"
  | "No chapters or bookmarks to define a range"
  | "Range too short to loop"
  | "Loop count: endless"
  | "Loop count: {}"
  | "This source doesn't support A-B loop"
  | "A-B loop failed: {}";

export type Osd = (text: OsdText, args?: TemplateArgs) => void;

/** The OSD; headless hosts refuse the display, which never fails the action itself. */
export const osdOf =
  (plugin: Plugin): Osd =>
  (text, args) =>
    void plugin.osd.show(text, args).catch(() => {});
