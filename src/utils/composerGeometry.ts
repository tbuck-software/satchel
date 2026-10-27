export type ResizeDirection = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export interface ComposerRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Default dock size yields to the available space on short or narrow windows. */
export function dockComposerRect(
  bounds: { width: number; height: number },
  size: { width: number; height: number },
  minimum: { width: number; height: number },
  gap: number,
): ComposerRect {
  const width = Math.min(bounds.width, Math.max(minimum.width, Math.min(size.width, bounds.width - gap * 2)));
  const height = Math.min(bounds.height, Math.max(minimum.height, Math.min(size.height, bounds.height - gap * 2)));
  return {
    x: bounds.width - width - Math.min(gap, (bounds.width - width) / 2),
    y: bounds.height - height - Math.min(gap, (bounds.height - height) / 2),
    width,
    height,
  };
}

export function fitComposerRect(
  rect: ComposerRect,
  bounds: { width: number; height: number },
  minimum: { width: number; height: number },
): ComposerRect {
  const width = Math.min(bounds.width, Math.max(minimum.width, rect.width));
  const height = Math.min(bounds.height, Math.max(minimum.height, rect.height));
  return { x: clamp(rect.x, 0, bounds.width - width), y: clamp(rect.y, 0, bounds.height - height), width, height };
}

export function resizeComposerRect(
  start: ComposerRect,
  direction: ResizeDirection,
  deltaX: number,
  deltaY: number,
  bounds: { width: number; height: number },
  minimum: { width: number; height: number },
): ComposerRect {
  // The viewport wins over the nominal minimum on very small windows.
  minimum = { width: Math.min(minimum.width, bounds.width), height: Math.min(minimum.height, bounds.height) };
  let { x, y, width, height } = start;

  if (direction.includes('e')) {
    width = clamp(start.width + deltaX, minimum.width, bounds.width - start.x);
  }
  if (direction.includes('s')) {
    height = clamp(start.height + deltaY, minimum.height, bounds.height - start.y);
  }
  if (direction.includes('w')) {
    x = clamp(start.x + deltaX, 0, start.x + start.width - minimum.width);
    width = start.width + start.x - x;
  }
  if (direction.includes('n')) {
    y = clamp(start.y + deltaY, 0, start.y + start.height - minimum.height);
    height = start.height + start.y - y;
  }

  return { x, y, width, height };
}

function clamp(value: number, minimum: number, maximum: number) {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}
