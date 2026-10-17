import { describe, expect, it } from 'vitest';
import { resizeComposerRect, type ResizeDirection } from './composerGeometry';

const start = { x: 100, y: 80, width: 600, height: 500 };
const bounds = { width: 1200, height: 900 };
const minimum = { width: 520, height: 450 };

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
      y: 130,
      width: 520,
      height: 450,
    });
    expect(resizeComposerRect(start, 'se', 900, 900, bounds, minimum)).toEqual({
      x: 100,
      y: 80,
      width: 1100,
      height: 820,
    });
  });
});
