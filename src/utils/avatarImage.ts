/** The edge of a stored account picture, in pixels. */
export const AVATAR_PIXELS = 128;

/**
 * Turn an image file into a square, centre-cropped data URL of at most
 * `AVATAR_PIXELS` per side. WebP where the engine can encode it, JPEG where
 * it cannot (older WebKit). Throws if the file is not a readable image; the
 * caller has one message for every way that can fail.
 */
export async function avatarDataUrl(file: Blob, pixels = AVATAR_PIXELS): Promise<string> {
  const bitmap = await createImageBitmap(file);
  try {
    const side = Math.min(bitmap.width, bitmap.height);
    if (side <= 0) throw new RangeError();
    const edge = Math.min(pixels, side);
    const canvas = document.createElement('canvas');
    canvas.width = edge;
    canvas.height = edge;
    const context = canvas.getContext('2d');
    if (!context) throw new TypeError();
    context.imageSmoothingQuality = 'high';
    context.drawImage(
      bitmap,
      (bitmap.width - side) / 2,
      (bitmap.height - side) / 2,
      side,
      side,
      0,
      0,
      edge,
      edge,
    );
    const webp = canvas.toDataURL('image/webp', 0.86);
    return webp.startsWith('data:image/webp') ? webp : canvas.toDataURL('image/jpeg', 0.88);
  } finally {
    bitmap.close();
  }
}
