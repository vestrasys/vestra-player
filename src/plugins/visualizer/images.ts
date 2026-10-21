/** What the player accepts for a runtime image (reference §6.1). */
export const MAX_IMAGE_BYTES = 16 * 1024 * 1024;
export const MAX_IMAGE_SIDE = 4096;

export type ImageInfo = { type: "png" | "jpeg"; width: number; height: number };

/**
 * The type and size of a PNG or JPEG from its bytes (the player judges by
 * content, not by extension); `null` for anything else or a header too
 * short to tell.
 */
export function imageInfo(bytes: Uint8Array): ImageInfo | null {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  // PNG: signature, then the IHDR chunk (width and height big-endian).
  const PNG = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (bytes.length >= 24 && PNG.every((b, i) => bytes[i] === b)) {
    if (String.fromCharCode(...bytes.subarray(12, 16)) !== "IHDR") return null;
    return { type: "png", width: view.getUint32(16), height: view.getUint32(20) };
  }
  // JPEG: SOI, then segments until a start-of-frame marker carries the size.
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) return null;
  let at = 2;
  while (at + 4 <= bytes.length) {
    if (bytes[at] !== 0xff) return null;
    const marker = bytes[at + 1]!;
    // Fill bytes and stand-alone markers carry no length.
    if (marker === 0xff) {
      at += 1;
      continue;
    }
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
      at += 2;
      continue;
    }
    const length = view.getUint16(at + 2);
    const frame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (frame) {
      if (at + 9 > bytes.length) return null;
      return { type: "jpeg", height: view.getUint16(at + 5), width: view.getUint16(at + 7) };
    }
    if (marker === 0xda || marker === 0xd9 || length < 2) return null;
    at += 2 + length;
  }
  return null;
}

/** Why the player would refuse this image, or `null` when it is fine. */
export function imageProblem(bytes: Uint8Array): "type" | "size" | null {
  const info = imageInfo(bytes);
  if (!info || info.width < 1 || info.height < 1) return "type";
  if (bytes.length > MAX_IMAGE_BYTES || info.width > MAX_IMAGE_SIDE || info.height > MAX_IMAGE_SIDE) return "size";
  return null;
}

/** The file extension for this image's real type. */
export const extensionOf = (info: ImageInfo) => (info.type === "png" ? "png" : "jpg");
