import { describe, expect, it } from 'vitest';
import { placeMenu, placePopoverAgainstAnchor, placePopoverBesideAnchor } from './popoverPlacement';

const viewport = { width: 1200, height: 800 };

describe('popover placement', () => {
  it('opens upward from a trigger sitting low in the window', () => {
    // The composer signature: its switcher lives just above the send bar.
    const placement = placePopoverAgainstAnchor({ top: 720, bottom: 744, right: 900 }, viewport);

    expect(placement.bottom).toBe('86px');
    expect(placement.top).toBeUndefined();
  });

  it('never claims more height than the chosen side actually has', () => {
    const placement = placePopoverAgainstAnchor({ top: 720, bottom: 744, right: 900 }, viewport);

    // Above the trigger: 720px minus gap and viewport margin.
    expect(placement.maxHeight).toBe('704px');
    const consumed = 800 - Number.parseInt(placement.bottom!, 10) - Number.parseInt(placement.maxHeight, 10);
    expect(consumed).toBeGreaterThanOrEqual(0);
  });

  it('flips below when the trigger sits near the top edge', () => {
    const placement = placePopoverAgainstAnchor({ top: 40, bottom: 64, right: 900 }, viewport);

    expect(placement.top).toBe('70px');
    expect(placement.bottom).toBeUndefined();
    expect(placement.maxHeight).toBe('720px');
  });

  it('keeps a usable height in a viewport too short for either side', () => {
    const placement = placePopoverAgainstAnchor(
      { top: 100, bottom: 124, right: 300 },
      { width: 400, height: 220 },
    );

    // 76px of real room below, 84px above — neither is usable, so the floor
    // applies and the popover scrolls internally instead of collapsing.
    expect(placement.maxHeight).toBe('140px');
  });

  it('keeps the popover off the right edge when the trigger overhangs it', () => {
    const placement = placePopoverAgainstAnchor({ top: 400, bottom: 424, right: 1400 }, viewport);

    expect(placement.right).toBe('10px');
  });

  it('aligns the popover with the trigger otherwise', () => {
    const placement = placePopoverAgainstAnchor({ top: 400, bottom: 424, right: 900 }, viewport);

    expect(placement.right).toBe('300px');
  });
});

describe('beside-anchor placement', () => {
  const size = { width: 320, height: 260 };

  it('opens to the right of an anchor with room to spare', () => {
    const placement = placePopoverBesideAnchor(
      { left: 400, top: 300, right: 500, bottom: 320 },
      viewport,
      size,
    );

    expect(placement.left).toBe('508px');
    expect(placement.top).toBe('300px');
  });

  it('flips to the left when the anchor hugs the right edge', () => {
    const placement = placePopoverBesideAnchor(
      { left: 1000, top: 300, right: 1180, bottom: 320 },
      viewport,
      size,
    );

    // 1000 - 8 gap - 320 width.
    expect(placement.left).toBe('672px');
  });

  it('clamps upward for an anchor near the bottom edge', () => {
    const placement = placePopoverBesideAnchor(
      { left: 400, top: 740, right: 500, bottom: 760 },
      viewport,
      size,
    );

    // 800 viewport - 10 margin - 260 height.
    expect(placement.top).toBe('530px');
  });

  it('stays inside a viewport narrower than both sides', () => {
    const placement = placePopoverBesideAnchor(
      { left: 150, top: 100, right: 250, bottom: 120 },
      { width: 400, height: 800 },
      size,
    );

    expect(placement.left).toBe('70px');
  });

  it('caps the height to the viewport and lets the card scroll', () => {
    const placement = placePopoverBesideAnchor(
      { left: 400, top: 100, right: 500, bottom: 120 },
      { width: 1200, height: 240 },
      size,
    );

    expect(placement.maxHeight).toBe('220px');
    expect(placement.top).toBe('10px');
  });
});

describe('menu placement', () => {
  const point = (x: number, y: number) => ({ left: x, top: y, right: x, bottom: y });
  const size = { width: 232, height: 320 };

  it('drops a right-click menu down and to the right of the pointer', () => {
    const placement = placeMenu(point(300, 200), viewport, size);

    expect(placement).toMatchObject({ left: '300px', top: '200px', side: 'right' });
  });

  it('flips to the left of a pointer near the right edge', () => {
    const placement = placeMenu(point(1150, 200), viewport, size);

    expect(placement).toMatchObject({ left: '918px', side: 'left' });
  });

  it('grows upward from a pointer near the bottom, ending at the click', () => {
    const placement = placeMenu(point(300, 700), viewport, size);

    expect(placement.top).toBe('380px');
  });

  it('shrinks rather than overflowing a short window', () => {
    const placement = placeMenu(point(300, 40), { width: 1200, height: 260 }, size);

    expect(placement).toMatchObject({ top: '40px', maxHeight: '212px' });
  });

  it('opens a submenu flush with its row, past the parent menu', () => {
    const row = { left: 300, top: 400, right: 532, bottom: 428 };

    const placement = placeMenu(row, viewport, { width: 224, height: 180 }, { gap: 4 });

    expect(placement).toMatchObject({ left: '536px', top: '400px' });
  });
});
