// Mail renders in an iframe. The reader supplies its canvas as a custom property;
// content colours are mapped to concrete hex values.
interface RgbColor {
  r: number;
  g: number;
  b: number;
  a: number;
}

const NAMED_COLORS: Record<string, RgbColor> = {
  black: { r: 0, g: 0, b: 0, a: 1 },
  white: { r: 255, g: 255, b: 255, a: 1 },
  blue: { r: 0, g: 0, b: 255, a: 1 },
  purple: { r: 128, g: 0, b: 128, a: 1 },
  red: { r: 255, g: 0, b: 0, a: 1 },
  green: { r: 0, g: 128, b: 0, a: 1 },
  gray: { r: 128, g: 128, b: 128, a: 1 },
  grey: { r: 128, g: 128, b: 128, a: 1 },
  transparent: { r: 0, g: 0, b: 0, a: 0 },
};

const DARK_CANVAS: RgbColor = { r: 36, g: 38, b: 43, a: 1 };

function parseChannel(value: string): number {
  if (value.endsWith('%')) {
    return Math.round(Math.min(100, Math.max(0, Number.parseFloat(value))) * 2.55);
  }
  return Math.min(255, Math.max(0, Number.parseFloat(value)));
}

function parseCssColor(value: string): RgbColor | null {
  const normalized = value.trim().toLowerCase();
  if (NAMED_COLORS[normalized]) return NAMED_COLORS[normalized];

  const hex = normalized.match(/^#([0-9a-f]{3,8})$/i)?.[1];
  if (hex) {
    if (hex.length === 3 || hex.length === 4) {
      const [r, g, b, a] = hex.split('');
      return {
        r: Number.parseInt(r! + r!, 16),
        g: Number.parseInt(g! + g!, 16),
        b: Number.parseInt(b! + b!, 16),
        a: a === undefined ? 1 : Number.parseInt(a + a, 16) / 255,
      };
    }
    if (hex.length === 6 || hex.length === 8) {
      return {
        r: Number.parseInt(hex.slice(0, 2), 16),
        g: Number.parseInt(hex.slice(2, 4), 16),
        b: Number.parseInt(hex.slice(4, 6), 16),
        a: hex.length === 8 ? Number.parseInt(hex.slice(6, 8), 16) / 255 : 1,
      };
    }
  }

  const rgb = normalized.match(/^rgba?\(\s*([^,]+),\s*([^,]+),\s*([^,)]+)(?:,\s*([^)]+))?\s*\)$/);
  if (!rgb) return null;
  // Groups 1-3 are required by the pattern above; only the alpha group is optional.
  const [, r, g, b, alpha] = rgb;
  return {
    r: parseChannel(r ?? ''),
    g: parseChannel(g ?? ''),
    b: parseChannel(b ?? ''),
    a:
      alpha === undefined
        ? 1
        : Math.min(1, Math.max(0, Number.parseFloat(alpha))),
  };
}

function linearChannel(channel: number): number {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function luminance(color: RgbColor): number {
  return 0.2126 * linearChannel(color.r) + 0.7152 * linearChannel(color.g) + 0.0722 * linearChannel(color.b);
}

function contrast(a: RgbColor, b: RgbColor): number {
  const bright = Math.max(luminance(a), luminance(b));
  const dark = Math.min(luminance(a), luminance(b));
  return (bright + 0.05) / (dark + 0.05);
}

function mixWithWhite(color: RgbColor, amount: number): RgbColor {
  return {
    r: Math.round(color.r + (255 - color.r) * amount),
    g: Math.round(color.g + (255 - color.g) * amount),
    b: Math.round(color.b + (255 - color.b) * amount),
    a: color.a,
  };
}

function toHex(color: RgbColor): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) => Math.round(channel).toString(16).padStart(2, '0'))
    .join('')}`;
}

export function readableDarkModeTextColor(value: string): string | null {
  const parsed = parseCssColor(value);
  if (!parsed || parsed.a < 0.1) return null;
  if (contrast(parsed, DARK_CANVAS) >= 4.8) return toHex(parsed);

  for (let amount = 0.08; amount <= 1; amount += 0.08) {
    const candidate = mixWithWhite(parsed, amount);
    if (contrast(candidate, DARK_CANVAS) >= 4.8) return toHex(candidate);
  }
  return '#d7dae0';
}

export function darkModeBackgroundColor(value: string): string | null {
  const parsed = parseCssColor(value);
  if (!parsed || parsed.a < 0.1) return null;
  const lightness = luminance(parsed);
  // Neutral sender canvases belong to the reader, including dark greys. Keep
  // coloured panels separate so banners and call-to-action fills survive.
  const chroma = Math.max(parsed.r, parsed.g, parsed.b) - Math.min(parsed.r, parsed.g, parsed.b);
  if (chroma <= 18) return 'var(--satchel-email-canvas)';
  if (lightness > 0.22) {
    return toHex({
      r: Math.round(parsed.r * 0.42),
      g: Math.round(parsed.g * 0.42),
      b: Math.round(parsed.b * 0.42),
      a: parsed.a,
    });
  }
  return toHex(parsed);
}

const COLOR_VALUE = '(?:#[0-9a-f]{3,8}|rgba?\\([^\\)]*\\)|black|white|blue|purple|red|green|gr[ae]y|transparent)';

export function rewriteCssColorsForDark(css: string): string {
  const declaration = new RegExp(
    `(^|[;{]\\s*)(color|background-color|background)\\s*:\\s*(${COLOR_VALUE})(\\s*!important)?(?=\\s*(?:[;}]|$))`,
    'gi',
  );
  return css.replace(declaration, (full, prefix: string, property: string, value: string, priority = '') => {
    const mapped = property.toLowerCase() === 'color'
      ? readableDarkModeTextColor(value)
      : darkModeBackgroundColor(value);
    return mapped ? `${prefix}${property}: ${mapped}${priority}` : full;
  });
}

export function isDesignedEmailMarkup(html: string): boolean {
  return /<table\b|\brole\s*=\s*["']?presentation\b/i.test(html);
}

export function responsiveEmailTableWidth(
  widthAttribute: string | null,
  inlineWidth: string,
): string | null {
  const declaredWidth = inlineWidth.trim() || widthAttribute?.trim() || '';
  const fixedWidth = declaredWidth.match(/^(\d+(?:\.\d+)?)\s*(?:px)?$/i);
  if (!fixedWidth) return null;

  // Group 1 is required by the pattern above.
  const pixels = Number.parseFloat(fixedWidth[1] ?? '');
  if (!Number.isFinite(pixels) || pixels <= 0) return null;
  return `min(100%, ${pixels}px)`;
}
