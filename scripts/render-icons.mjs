#!/usr/bin/env node
/**
 * Renders every Satchel icon from the two brand SVGs:
 *
 *   public/branding/satchel-smile-check.svg  the app icon: mark on its tile
 *   public/branding/satchel-mark.svg         the mark alone, transparent
 *
 * Change one of them and run `pnpm icons` (or `make icons`); nothing else is
 * drawn by hand. Outputs:
 *
 * - App icons in `src-tauri/icons` (desktop, iOS, Android mipmaps) through
 *   `tauri icon`, written to a scratch directory first so that it cannot touch
 *   the Android project's adaptive icon. iOS gets the tile colour behind the
 *   mark, because iOS fills transparency with white.
 * - The web favicon and apple-touch icon in `public/`.
 * - The monochrome status icons: `trayTemplate.png` for the menu bar / tray
 *   (macOS reads only its alpha) and `ic_notification.png` in every Android
 *   density (Android reads only its alpha).
 * - The Android launcher: `ic_launcher_mark.xml`, the mark as a vector inside
 *   the adaptive icon's safe zone, and the tile colour in `colors.xml`.
 */
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from '@playwright/test';

const root = fileURLToPath(new URL('..', import.meta.url));
const APP_ICON = 'public/branding/satchel-smile-check.svg';
const MARK = 'public/branding/satchel-mark.svg';
const ICONS = 'src-tauri/icons';
const ANDROID_RES = 'src-tauri/gen/android/app/src/main/res';

const appIcon = readFileSync(join(root, APP_ICON), 'utf8');
const mark = readFileSync(join(root, MARK), 'utf8');

/** @param {string} svg @param {string} id @param {string} name */
function attribute(svg, id, name) {
  const element = new RegExp(`<[a-z]+\\b[^>]*\\bid="${id}"[^>]*>`, 's').exec(svg)?.[0];
  const value = element && new RegExp(`\\b${name}="([^"]*)"`).exec(element)?.[1];
  if (!value) throw new Error(`${APP_ICON}: #${id} has no ${name}`);
  return value;
}

/** The tile colour as six hex digits, without `#`. */
const tile = attribute(appIcon, 'background', 'fill').replace(/^#/, '').toUpperCase();

/** @param {string} path @param {string | Buffer} content */
function write(path, content) {
  const target = join(root, path);
  if (typeof content === 'string' && existsSync(target) && readFileSync(target, 'utf8') === content) return;
  writeFileSync(target, content);
  console.log(path);
}

/**
 * Whether two PNGs hold the same pixels. PNG encoders differ between releases
 * of Chromium and of `tauri icon`; an unchanged icon should not show up as a
 * change.
 * @param {import('@playwright/test').Page} tab @param {Buffer} next @param {Buffer} current
 */
function samePixels(tab, next, current) {
  return tab.evaluate(async ([a, b]) => {
    /** @param {string} base64 */
    const pixels = async (base64) => {
      const bitmap = await createImageBitmap(await (await fetch(`data:image/png;base64,${base64}`)).blob());
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = /** @type {OffscreenCanvasRenderingContext2D} */ (canvas.getContext('2d'));
      context.drawImage(bitmap, 0, 0);
      return { width: bitmap.width, data: context.getImageData(0, 0, bitmap.width, bitmap.height).data };
    };
    const [x, y] = await Promise.all([pixels(a), pixels(b)]);
    return x.width === y.width && x.data.length === y.data.length && x.data.every((value, index) => value === y.data[index]);
  }, [next.toString('base64'), current.toString('base64')]);
}

/** @param {import('@playwright/test').Page} tab @param {string} path @param {Buffer} png */
async function unchanged(tab, path, png) {
  const target = join(root, path);
  return existsSync(target) && samePixels(tab, png, readFileSync(target));
}

// --- App icons -------------------------------------------------------------

/** The files of the `tauri icon` set the app ships; the Windows Store logos are not among them. */
const SHIPPED = /^(32x32|128x128|128x128@2x|icon)\.png$|^icon\.(icns|ico)$|^ios\/.+\.png$|^android\/mipmap-[a-z]+\/.+\.png$/;

/** @param {string} dir @returns {string[]} */
function filesUnder(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? filesUnder(path) : [path];
  });
}

/**
 * The set is replaced as a whole, and only when its 512 px `icon.png` shows
 * different pixels: `.icns` and `.ico` cannot be compared that way, and
 * `tauri icon` releases differ in encoding and in some Android sizes.
 * @param {import('@playwright/test').Page} tab
 */
async function renderAppIcons(tab) {
  const scratch = mkdtempSync(join(tmpdir(), 'satchel-icons-'));
  try {
    const tauri = spawnSync('pnpm', ['exec', 'tauri', 'icon', APP_ICON, '--ios-color', `#${tile}`, '-o', scratch], {
      cwd: root,
      stdio: ['ignore', 'ignore', 'inherit'],
    });
    if (tauri.status !== 0) throw new Error(`tauri icon exited with ${tauri.status}`);
    if (await unchanged(tab, `${ICONS}/icon.png`, readFileSync(join(scratch, 'icon.png')))) return;
    for (const file of filesUnder(scratch)) {
      const name = relative(scratch, file);
      if (!SHIPPED.test(name)) continue;
      cpSync(file, join(root, ICONS, name));
      console.log(`${ICONS}/${name}`);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

// --- Rasters drawn by the browser ------------------------------------------

/** Notification icons are 24 dp with a 2 dp margin (Material status-bar grid). */
const ANDROID_DENSITIES = { mdpi: 1, hdpi: 1.5, xhdpi: 2, xxhdpi: 3, xxxhdpi: 4 };

/** @type {{ path: string, svg: string, size: number, inset: number, color?: string }[]} */
const rasters = [
  { path: 'public/favicon-32x32.png', svg: appIcon, size: 32, inset: 0 },
  { path: 'public/apple-touch-icon.png', svg: appIcon, size: 180, inset: 0 },
  // 32 px drawn at 18 pt high in the menu bar; a 1 px margin each side.
  { path: `${ICONS}/trayTemplate.png`, svg: mark, size: 32, inset: 1, color: '#000000' },
  ...Object.entries(ANDROID_DENSITIES).map(([density, scale]) => ({
    path: `${ANDROID_RES}/drawable-${density}/ic_notification.png`,
    svg: mark,
    size: 24 * scale,
    inset: 2 * scale,
    color: '#FFFFFF',
  })),
];

/**
 * The SVG centred in a square of `size` with `inset` around it, the mark
 * recoloured to `color` when one is given.
 * @param {{ svg: string, size: number, inset: number, color?: string }} raster
 */
function page({ svg, size, inset, color }) {
  let source = svg.replace(/<title[^>]*>.*?<\/title>/s, '');
  // The envelope's fill, not the mask's white (that one has to stay white).
  if (color) source = source.replace(/fill="white"(\s+mask=)/, `fill="${color}"$1`);
  const side = size - 2 * inset;
  source = source.replace(/<svg\b[^>]*>/, (tag) =>
    tag.replace(/\s(?:width|height)="[^"]*"/g, '').replace('<svg', `<svg width="${side}" height="${side}"`),
  );
  return `<!doctype html><html><body style="margin:0;background:transparent">
    <div id="icon" style="width:${size}px;height:${size}px;display:grid;place-items:center">${source}</div>
  </body></html>`;
}

const browser = await chromium.launch();
try {
  const tab = await browser.newPage({ deviceScaleFactor: 1 });
  await renderAppIcons(tab);
  for (const raster of rasters) {
    await tab.setContent(page(raster));
    const png = await tab.locator('#icon').screenshot({ omitBackground: true });
    if (!(await unchanged(tab, raster.path, png))) write(raster.path, png);
  }
} finally {
  await browser.close();
}

// --- Android launcher --------------------------------------------------------

/** SVG path data in the comma-paired form Android Studio writes. @param {string} d */
function androidPath(d) {
  return (d.match(/[A-Za-z][^A-Za-z]*/g) ?? [])
    .map((segment) => {
      const numbers = segment.slice(1).trim().split(/[\s,]+/).filter(Boolean);
      const pairs = [];
      for (let index = 0; index < numbers.length; index += 2) pairs.push(numbers.slice(index, index + 2).join(','));
      return segment[0] + pairs.join(' ');
    })
    .join(' ');
}

const envelope = androidPath(attribute(appIcon, 'envelope', 'd'));
const smile = androidPath(attribute(appIcon, 'smile-check', 'd'));
const smileWidth = attribute(appIcon, 'smile-check', 'stroke-width');

write(
  `${ANDROID_RES}/drawable/ic_launcher_mark.xml`,
  `<?xml version="1.0" encoding="utf-8"?>
<!--
  Generated by scripts/render-icons.mjs (pnpm icons) from
  ${APP_ICON}: the mark scaled into the adaptive icon's 66dp safe zone
  (farthest corner at 28dp from centre). The launcher masks the 108dp layer to
  a circle, squircle or rounded square; anything outside the safe zone can be
  cut away.
-->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <group
        android:scaleX="0.07946"
        android:scaleY="0.07946"
        android:translateX="13.3165"
        android:translateY="13.3165">
        <path
            android:fillColor="#FFFFFFFF"
            android:pathData="${envelope}" />
        <path
            android:pathData="${smile}"
            android:strokeColor="@color/satchel_icon_tile"
            android:strokeWidth="${smileWidth}"
            android:strokeLineCap="round"
            android:strokeLineJoin="round" />
    </group>
</vector>
`,
);

const colorsPath = `${ANDROID_RES}/values/colors.xml`;
const colors = readFileSync(join(root, colorsPath), 'utf8');
const tileColor = /(<color name="satchel_icon_tile">)#[0-9A-F]{8}(<\/color>)/i;
if (!tileColor.test(colors)) throw new Error(`${colorsPath}: no satchel_icon_tile colour`);
write(colorsPath, colors.replace(tileColor, `$1#FF${tile}$2`));
