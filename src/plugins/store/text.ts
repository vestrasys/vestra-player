import type { I18n, ListStyle, TextArg, TextValue } from "@vestra-player/sdk";
import type { CapabilityText } from "./labels.ts";

/**
 * Every sentence the store builds at run time is a `Msg`: an English
 * template from the tables below and its arguments. Arguments are data
 * (names, versions, addresses, error messages from the player), another
 * `Msg`, or a list of them. Nothing is translated here: the player
 * translates. `argOf()` / `textOf()` turn a `Msg` into a text value
 * (`plugin.i18n.text`) for the windows, the OSD and the confirmations;
 * `vestra-plugin i18n extract` reads the templates from these `as const`
 * tables through the type of `Msg.text`.
 */

/** Templates of the windows' status line, notices and labels (also shown on the OSD). */
export const WINDOW = [
  // Status line: refusals and results.
  "Another install is in progress; wait for it to finish",
  "This system does not support confirmation dialogs; cannot continue",
  "Another dialog is open; close it first",
  "Couldn't open the link: {}",
  "This kind of link cannot be opened",
  "The downloaded file does not match the repository's description (size); deleted",
  "The downloaded file does not match the repository's description (checksum); deleted",
  "The plugin package exceeds 64 MB; download stopped",
  "The download address is insecure or redirects too often; stopped",
  "Download cancelled",
  "Couldn't write the download file: {}",
  "Download failed: {}",
  "This is not a valid plugin package: {}",
  "This plugin needs Vestra Player ≥ {} (current {})",
  "This plugin needs a newer Vestra Player (current {})",
  "“{}” has the same id as a built-in plugin and cannot be installed",
  "This plugin has the same id as a built-in plugin and cannot be installed",
  "Plugins are being installed or cleaned up; try again later",
  "The Plugin Store lacks the “Manage plugins” permission",
  "This plugin no longer exists",
  "The plugin package does not match the repository's description (id); not installed",
  "The plugin package does not match the repository's description (version); not installed",
  "The plugin package does not match the repository's description (permissions); not installed",
  "You can only install from https:// links",
  "Not installed",
  "Installed {} {}",
  "Updated {} to {}",
  "Uninstalled {} (data deleted)",
  "Uninstalled {} (data kept)",
  "Built-in plugins cannot be uninstalled",
  "Enabled “{}” again",
  "Couldn't enable “{}”: {}",
  // An install or uninstall that did not simply finish.
  "Can no longer cancel; the installation continues",
  "Can no longer cancel; the uninstallation continues",
  "Installation failed; the previous version was restored",
  "Installation cancelled",
  "Uninstallation cancelled",
  "Installation failed: {}",
  "Uninstallation failed: {}",
  // Work in progress (next to the status line).
  "Downloading {} {}",
  "Downloading {} {} ({}%)",
  "Downloading the plugin package",
  "Downloading the plugin package ({}%)",
  "Checking {}…",
  "Checking the plugin package…",
  "Installing {}…",
  "Uninstalling {}…",
  "Installing… (preparing)",
  "Installing… (activating)",
  "Uninstalling… (preparing)",
  "Uninstalling… (activating)",
  // Repositories (status line, settings page). The first argument is the repository's label.
  "Updating {}…",
  "{0}: up to date at {1}",
  "{0}: updated at {1}",
  "{0}: data from {1}",
  "The official repository has no signature file; this update was ignored",
  "The official repository's signature check failed (it may have been tampered with); this update was ignored",
  "The official repository returned an old index; this update was ignored",
  "{} uses a newer format; please update Vestra Player",
  "{}'s index is malformed",
  "{0}'s index is malformed; showing the data from {1}",
  "{0} returned error {1}",
  "{0} returned error {1}; showing the data from {2}",
  "{}'s index exceeds 4 MB; refused",
  "{0}'s index exceeds 4 MB; refused; showing the data from {1}",
  "{0}: {1}",
  "{0}: {1}; showing the data from {2}",
  "Can't connect to {0} ({1}); its plugins are not available for now",
  "Can't connect to {0} ({1}); showing the cache from {2}",
  // Settings page.
  "This repository has already been added",
  "At most {} repositories can be added",
  "Reading {}…",
  "Couldn't read this repository: {}. Check the address (the repository's root address or the full address of index.json)",
  "Added {}",
  "Removed {}",
  // The list.
  "No plugins match",
  "All plugins are up to date",
  "No plugins installed yet",
  "No plugins installed besides the built-in ones, which are hidden",
  "Reading repositories…",
  "Can't reach the repositories. Showing nothing until you're online; click Refresh to retry.",
  "The repositories have no plugins yet",
  "Every plugin from the repositories is installed",
  "{} · {}",
  "Installed",
  "Update → {}",
  "Revoked",
  "Installed from {}",
  "Incompatible",
  "Cannot be installed",
  "Disabled",
  "Failed",
  // A row's small tags: a development load (--plugin), what a running plugin was refused.
  "Dev",
  "Warning",
  "Development load",
  // A button and its … menu.
  "Install",
  "Update",
  "Enable",
  "Disable",
  "Reload",
  "Open Data Folder",
  "Working…",
  // Enable, disable, reload, the data folder.
  "Enabled “{}”",
  "Disabled “{}”",
  "Reloaded “{}”",
  "Couldn't disable “{}”: {}",
  "Couldn't reload “{}”: {}",
  "Built-in plugins cannot be disabled",
  "Disabled “{}” until the player quits",
  "“{}” is being installed or reloaded; try again later",
  "“{}” is disabled; enable it instead of reloading it",
  "Its data folder does not exist yet",
  "Couldn't open the data folder: {}",
  // The detail: buttons.
  "Install {}",
  "Update to {}",
  "Enable Anyway…",
  "Replace…",
  "Uninstall…",
  "Load",
  "Show Images",
  "Retry",
  // The detail: notices.
  "This version has been revoked: {}.",
  "This version has been revoked: {}. You confirmed the risk and enabled it again; the store will not disable it again.",
  "This version has been revoked: {}. The store has disabled it.",
  "Unofficial repository: not reviewed by Vestra Player; plugins may read your files or use the network",
  "This is a test repository on this machine, without encryption",
  "Installed {1} from {0}. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data; the current data is kept",
  "Same id as a built-in plugin of the player; cannot be installed from here",
  "Built-in plugins are updated with the player and cannot be uninstalled or disabled",
  "The repository no longer offers this plugin; the store no longer checks it for updates (it still checks for revocations)",
  "Its repository was removed from the store; the store no longer checks it for updates or revocations",
  "Sideloaded plugin: not reviewed; the store does not check it for updates",
  "This run uses the development-loaded version",
  "Loaded for development (--plugin) in this run: disabling it lasts until the player quits, and it cannot be uninstalled here",
  // What a running plugin was refused because another plugin holds it (PluginInfo.warnings); the argument is that plugin's name.
  "Its bottom bar is not shown: “{}” holds the bottom bar",
  "Its sidebar tab is not shown: “{}” holds that place",
  "Its sidebar tab is not shown: the sidebar is full",
  "Its backdrop is not shown: “{}” holds the backdrop",
  "Its panel is not shown: “{}” holds that place",
  "Its panel is not shown: that place is full",
  "Its Previous/Next is not used: “{}” provides them",
  "It does not open {}: “{}” does",
  "Part of it was refused because another plugin holds it ({})",
  "The description is on another website ({}); not loaded automatically",
  "Remote images blocked (unofficial source): {}",
  "From the installed version",
  // The detail: README and changelog.
  "Loading…",
  "Loading the description…",
  "Loading the changelog…",
  "The author provided no description",
  "The author provided no changelog",
  "The description is on another website; not loaded automatically",
  "The changelog is on another website; not loaded automatically",
  "Couldn't load the description: {}",
  "Couldn't load the changelog: {}",
  "Couldn't show the description: {}",
  "Couldn't show the changelog: {}",
  // The detail: versions and facts.
  "Current",
  "Current, {}",
  "Incompatible: {}",
  "Latest",
  "Version",
  "Updated",
  "Source",
  "License",
  "Code",
  "Open ↗",
  "Status",
  "Running",
  "Starting",
  "Stopped",
  "Failed: {}",
  "Package",
  "Data folder",
  "Data size",
  // The tag drop-down's first entry.
  "Tags: All",
  // A banner (the permissions are a list).
  "The new version needs new permissions: {}",
] as const;

/** Parts of other sentences (labels, reasons); their arguments are plain. */
export const PART = [
  "Official repository",
  "{} ({})",
  "Unnamed repository ({})",
  "Unofficial: {} ({})",
  "Unofficial: Unnamed repository ({})",
  "Vestra Player official repository",
  "Built-in",
  "Local file",
  "Link",
  "Plugin Workshop",
  "Repository {}",
  "Unknown source",
  "Unknown author",
  // Why a download or a repository failed.
  "Cancelled",
  "Only https:// addresses are supported",
  "Redirected to an insecure address",
  "Redirect without a target address ({})",
  "Too many redirects",
  "Size does not match the description",
  "File exceeds the size limit",
  "Checksum does not match the description",
  "Connection timed out",
  "Server not found",
  "Connection refused",
  "Connection reset",
  "Secure connection failed",
  "File exceeds 256 KB",
  "Not JSON",
  "The top level is not an object",
  "Wrong schema",
  "plugins is not an array",
  "revoked is not an array",
  "revoked has more than {} entries",
  // Why a version cannot be installed.
  "Needs a newer plugin API ({})",
  "Needs Vestra Player ≥ {} (current {})",
  "Revoked: {}",
  "This plugin has been revoked: {}",
  "“{}”: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed or loaded for testing",
  "This plugin: ids starting with vestra. are reserved for the player's built-in plugins and cannot be installed or loaded for testing",
  "Enter a repository address starting with https://",
  "The address cannot contain a user name or password",
  "The repository address is too long",
  // A capability the store does not know.
  "Unknown permission: {}",
] as const;

/** Only on the OSD or in confirmations, never in a window. */
export const HOST = [
  "Plugin Store: plugin updates available ({})",
  "Disabled the revoked plugin “{}”",
  // Confirmation: install from a repository.
  "Replace the installed “{}”?",
  "{1}\n\nWill install {2} from the official repository.\n\n{3}",
  "{1}\n\nWill install {2} from the unofficial repository.\n\n{4}\n\n{3}",
  "Update “{}” to {}? The new version needs more permissions",
  "Added:\n{2}\n\nCurrent: {3}.\n\nA plugin is a complete program: the permissions above only limit what it can do with the player, not reading and writing files or using the network.",
  "Update “{}” to {}?",
  "Current: {2}.",
  "Install “{}” {} (reinstall)?",
  "Current: {2}.\n\nThis version needs more permissions:\n{3}",
  "Install “{}” {}?",
  "From the Vestra Player official repository, author {2}.\n\n{3}",
  "Install “{}” {} from an unofficial repository?",
  "{2}\n\n{3}",
  "{1}",
  "This plugin has already been installed from another source. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data. The original plugin's data is kept and is used again when you reinstall it from the original source.",
  // Parts of those confirmations.
  "The current “{}” {} comes from {}. Replacing it continues with the data from an earlier install from this source, if there was one, otherwise with new, empty data. The original plugin's data is kept and is used again when you reinstall it from the original source.",
  "Repository: {} ({}). This repository is not reviewed by Vestra Player; its plugins may read your files or use the network. Only install plugins from authors you trust.",
  "Repository: {} ({}). This repository is not reviewed by Vestra Player; its plugins may read your files or use the network. Only install plugins from authors you trust. This is a test repository on this machine, without encryption.",
  "It will be able to:\n{}\n\nA plugin is a complete program: the permissions above only limit what it can do with the player, not reading and writing files or using the network.",
  "Unknown",
  // The permission list: one line each, at most 20.
  "· {}",
  "· Uses none of the player's features",
  "{} in total",
  // Confirmation: a package from a file or a link.
  "Replace the installed version with the plugin package “{}” {}?",
  "{2}\n\n{3}\n\n{4}",
  "Install the plugin package “{}” {}?",
  "{2}\n\nCurrently installed: {3} (same source).\n\n{4}",
  "From the file {}; not reviewed by any repository and not updated automatically. Only install plugins from authors you trust.",
  "From the link {}; not reviewed by any repository and not updated automatically. Only install plugins from authors you trust.",
  // Confirmation: uninstall.
  "Uninstall “{}”?",
  "Its menus, windows and processing effects are removed immediately.",
  // Confirmations: repositories (settings page).
  "Add the unofficial repository “{}”?",
  "Add the unofficial repository “Unnamed repository”?",
  "Address: {1}\n\nIts plugins are not reviewed by Vestra Player and may read your files or use the network. The store shows this repository's plugins but does not judge for you whether they can be trusted.",
  "Address: {1}\n\nIts plugins are not reviewed by Vestra Player and may read your files or use the network. The store shows this repository's plugins but does not judge for you whether they can be trusted.\n\nThis is a test repository on this machine, without encryption.",
  "Remove the repository “{} ({})”?",
  "Remove the repository “Unnamed repository ({1})”?",
  "The store will stop reading this repository. Plugins installed from it stay, but the store no longer checks them for updates and revocations.",
  // Confirmations: revoked versions.
  "Disabled the revoked plugin “{}” {}",
  "Disabled {} revoked plugins",
  "The repository revoked these versions. Reasons:\n\n{2}\n\nTo keep using one anyway, choose “Enable Anyway…” on its page in the Plugin Store.",
  "“{}” {}: {}",
  "“{}” {}: {}\nYou can update to {} in the Plugin Store.",
  "{}\n\n{}",
  "Enable the revoked “{}” {} anyway?",
  "The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.",
  "The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.\n\nUpdating to {3} instead is recommended.",
  "“{}” {} has been revoked. Keep using it?",
  "It was just enabled again. The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.",
  "It was just enabled again. The repository's reason for revoking this version: {2}\n\nUsing it may be risky. Once you confirm, the store will not disable it automatically again, and its page keeps showing a warning.\n\nUpdating to {3} instead is recommended.",
] as const;

/** Button and check box labels of the confirmations (translated, never formatted). */
export const LABELS = [
  "Add",
  "Remove",
  "Replace",
  "Update",
  "Install",
  "Install Anyway",
  "Uninstall",
  "Also delete its settings and data?",
  "Open Plugin Store",
  "OK",
  "Enable Anyway",
  "Keep Using",
  "Disable",
] as const;

export type WindowTemplate = (typeof WINDOW)[number];
export type PartTemplate = (typeof PART)[number];
export type Template = WindowTemplate | PartTemplate | (typeof HOST)[number] | CapabilityText;
export type Label = (typeof LABELS)[number];

export type Arg = string | number | Msg | Many;
export interface Msg {
  readonly text: Template;
  readonly args: readonly Arg[];
}
/** Items joined the interface language's way (`plugin.i18n.list`). */
export interface Many {
  readonly items: readonly Arg[];
  readonly style: ListStyle;
}

export const msg = (text: Template, ...args: Arg[]): Msg => ({ text, args });

/** A plain value shown as it is (an error message from the player). */
export const raw = (value: string): Msg => ({ text: "{}" as Template, args: [value] });

export const list = (items: readonly Arg[], style: ListStyle = "list"): Many => ({ items, style });

/**
 * Items one after another with `between` (`"{} · {}"`, `"{}\n\n{}"`), as a
 * balanced tree so that it nests only log₂(n) deep (texts nest at most 8).
 */
export function joined(items: readonly Arg[], between: "{} · {}" | "{}\n\n{}"): Arg {
  if (items.length <= 1) return items[0] ?? "";
  const half = Math.ceil(items.length / 2);
  return msg(between, joined(items.slice(0, half), between), joined(items.slice(half), between));
}

const isMsg = (value: Msg | Many): value is Msg => "text" in value;
const isRaw = (value: Msg) => (value.text as string) === "{}";

/** Cut to `max` characters (the player's limits count characters). */
export function cut(text: string, max: number): string {
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

type Words = Pick<I18n, "text" | "list">;

/**
 * An argument as the player takes it: data as it is (strings cut to `max`
 * characters), a `Msg` as a text value, a `Many` as a list value.
 */
export function argOf(words: Words, value: Arg, max: number): TextArg {
  if (typeof value === "number") return value;
  if (typeof value === "string") return cut(value, max);
  if (!isMsg(value)) return words.list(value.items.map((each) => argOf(words, each, max)), value.style);
  if (isRaw(value)) return argOf(words, value.args[0] ?? "", max);
  return words.text(
    value.text,
    value.args.map((each) => argOf(words, each, max)),
  );
}

/** A `Msg` (or other argument) as window text (a text value, or data for a plain value); `null` is no text. */
export function textOf(words: Words, value: Arg | null, max = 1000): TextValue {
  if (value === null || value === "") return "";
  const shown = argOf(words, value, max);
  return typeof shown === "number" ? String(shown) : shown;
}
