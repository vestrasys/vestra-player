/**
 * The app's Info.plist (bundle-macos.mjs), apart so that the tests read it
 * without building a bundle (packaging/tests/file-types.test.mjs).
 */
import { macosDocumentTypes } from "./file-types.mjs";

export const BUNDLE_ID = "com.vestraplayer.player";
export const DISPLAY_NAME = "Vestra Player";

/** The Info.plist entries of a build of `version` for macOS `minimum` or later. */
export function infoPlistEntries(version, minimum) {
  return {
    CFBundleDevelopmentRegion: "en",
    CFBundleDisplayName: DISPLAY_NAME,
    CFBundleExecutable: "vestra-host",
    CFBundleIconFile: "AppIcon",
    CFBundleIdentifier: BUNDLE_ID,
    CFBundleInfoDictionaryVersion: "6.0",
    CFBundleName: DISPLAY_NAME,
    CFBundlePackageType: "APPL",
    CFBundleShortVersionString: version,
    CFBundleVersion: version,
    LSApplicationCategoryType: "public.app-category.video",
    LSMinimumSystemVersion: minimum,
    NSHighResolutionCapable: true,
    NSHumanReadableCopyright: `© ${new Date().getUTCFullYear()} Ziyang Song · GPL-3.0-or-later`,
    NSSupportsAutomaticGraphicsSwitching: true,
    // Finder "Open With" (file-types.mjs); the host takes the open-documents
    // events (src/host/unsafe/open).
    ...macosDocumentTypes(),
  };
}

/** XML property list of strings, booleans, arrays and dictionaries. */
export function plist(entries) {
  const escape = (text) =>
    String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;");
  const value = (item, indent) => {
    const inner = `${indent}  `;
    if (typeof item === "boolean") return `<${item}/>`;
    if (Array.isArray(item))
      return item.length
        ? `<array>\n${item.map((element) => `${inner}${value(element, inner)}`).join("\n")}\n${indent}</array>`
        : "<array/>";
    if (item && typeof item === "object")
      return `<dict>\n${dict(item, inner)}\n${indent}</dict>`;
    return `<string>${escape(item)}</string>`;
  };
  const dict = (object, indent) =>
    Object.entries(object)
      .map(
        ([key, item]) =>
          `${indent}<key>${escape(key)}</key>\n${indent}${value(item, indent)}`,
      )
      .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${dict(entries, "  ")}
</dict>
</plist>
`;
}

export const infoPlist = (version, minimum) =>
  plist(infoPlistEntries(version, minimum));
