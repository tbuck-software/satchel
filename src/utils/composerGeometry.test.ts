import { describe, expect, it } from 'vitest';
import { COMPOSER_DEFAULT_SIZE, COMPOSER_MINIMUM_SIZE, COMPOSER_DOCK_GAP, dockComposerRect, fitComposerRect, resizeComposerRect, type ResizeDirection } from './composerGeometry';

const start = { x: 100, y: 80, width: 600, height: 500 };
const bounds = { width: 1200, height: 900 };
const minimum = COMPOSER_MINIMUM_SIZE;

describe('resizeComposerRect', () => {
  it.each<[ResizeDirection, number, number, Partial<typeof start>]>([
    ['n', 0, -30, { y: 50, height: 530 }],
    ['e', 40, 0, { width: 640 }],
    ['s', 0, 40, { height: 540 }],
    ['w', -40, 0, { x: 60, width: 640 }],
    ['ne', 40, -30, { y: 50, width: 640, height: 530 }],
    ['se', 40, 40, { width: 640, height: 540 }],
    ['sw', -40, 40, { x: 60, width: 640, height: 540 }],
    ['nw', -40, -30, { x: 60, y: 50, width: 640, height: 530 }],
  ])('resizes from %s', (direction, deltaX, deltaY, expected) => {
    expect(resizeComposerRect(start, direction, deltaX, deltaY, bounds, minimum)).toMatchObject(
      expected,
    );
  });

  it('enforces minimum size and window bounds', () => {
    expect(resizeComposerRect(start, 'nw', 500, 500, bounds, minimum)).toEqual({
      x: 180,
      y: 80,
      width: 520,
      height: 500,
    });
    expect(resizeComposerRect(start, 'se', 900, 900, bounds, minimum)).toEqual({
      x: 100,
      y: 80,
      width: 1100,
      height: 820,
    });
  });
});

describe('composer docking', () => {
  const size = COMPOSER_DEFAULT_SIZE;
  it('opens at the bottom-right with the standard gap', () => {
    expect(dockComposerRect({ width: 1440, height: 900 }, size, minimum, COMPOSER_DOCK_GAP)).toEqual({ x: 802, y: 342, ...size });
    expect(dockComposerRect({ width: 1100, height: 700 }, size, minimum, COMPOSER_DOCK_GAP)).toEqual({ x: 462, y: 142, ...size });
  });
  it('reduces the gap before reducing the minimum size', () => {
    expect(dockComposerRect({ width: 1100, height: 520 }, size, minimum, COMPOSER_DOCK_GAP)).toEqual({ x: 462, y: 10, width: 620, height: 500 });
  });
  it('fits short windows even when they cannot hold the minimum height', () => {
    expect(dockComposerRect({ width: 540, height: 400 }, size, minimum, COMPOSER_DOCK_GAP)).toEqual({ x: 10, y: 0, width: 520, height: 400 });
    expect(fitComposerRect(start, { width: 500, height: 400 }, minimum)).toEqual({ x: 0, y: 0, width: 500, height: 400 });
  });
  it('keeps manually placed geometry while it fits', () => {
    expect(fitComposerRect(start, bounds, minimum)).toEqual(start);
  });
  it('can resize after fitting below the nominal minimum', () => {
    const small = { x: 0, y: 0, width: 500, height: 400 };
    expect(resizeComposerRect(small, 'nw', 100, 100, { width: 500, height: 400 }, minimum)).toEqual(small);
  });
});
