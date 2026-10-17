export type ResizeDirection = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw';

export interface ComposerRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function resizeComposerRect(
  start: ComposerRect,
  direction: ResizeDirection,
  deltaX: number,
  deltaY: number,
  bounds: { width: number; height: number },
  minimum: { width: number; height: number },
): ComposerRect {
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
