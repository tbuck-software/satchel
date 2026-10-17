export interface AnchorRect {
  top: number;
  bottom: number;
  right: number;
}

/** Full box of an anchor element or click point, in viewport coordinates. */
export interface AnchorBox {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface ViewportSize {
  width: number;
  height: number;
}

/** Fixed-position offsets, ready to bind as an inline style. */
export interface PopoverPlacement {
  right: string;
  top?: string;
  bottom?: string;
  maxHeight: string;
}

export interface PlacementOptions {
  /** Distance between anchor and popover. */
  gap?: number;
  /** Distance the popover keeps from the viewport edges. */
  margin?: number;
  /** The popover never shrinks below this, even in a cramped viewport. */
  minHeight?: number;
}

/**
 * Places a popover against its trigger in viewport coordinates.
 *
 * A menu positioned inside its own block is clipped by whatever scroll
 * container it lives in — for a trigger near the bottom of a scrolling pane,
 * the menu opens upward and loses its top rows. Anchoring against the
 * viewport instead needs three decisions: which side has more room, how tall
 * the popover may grow on that side, and how to keep it off the edges.
 */
export function placePopoverAgainstAnchor(
  anchor: AnchorRect,
  viewport: ViewportSize,
  options: PlacementOptions = {},
): PopoverPlacement {
  const gap = options.gap ?? 6;
  const margin = options.margin ?? 10;
  const minHeight = options.minHeight ?? 140;

  const spaceAbove = anchor.top - gap - margin;
  const spaceBelow = viewport.height - anchor.bottom - gap - margin;
  // Ties go up: triggers sitting low in a pane are the case this exists for.
  const placeAbove = spaceAbove >= spaceBelow;

  const right = `${Math.max(margin, viewport.width - anchor.right)}px`;
  const maxHeight = `${Math.max(minHeight, placeAbove ? spaceAbove : spaceBelow)}px`;

  return placeAbove
    ? { right, bottom: `${viewport.height - anchor.top + gap}px`, maxHeight }
    : { right, top: `${anchor.bottom + gap}px`, maxHeight };
}

export interface PopoverSize {
  width: number;
  height: number;
}

/** Fixed-position offsets for a card placed beside its anchor. */
export interface SidePlacement {
  left: string;
  top: string;
  maxHeight: string;
}

/**
 * Places a card beside its anchor — calendar chips and grid slots. The card
 * prefers the side with more room (right on ties), stays aligned with the
 * anchor's top edge, and is clamped to the viewport on both axes so a chip in
 * the bottom-right corner never pushes the card off screen.
 */
export function placePopoverBesideAnchor(
  anchor: AnchorBox,
  viewport: ViewportSize,
  size: PopoverSize,
  options: PlacementOptions = {},
): SidePlacement {
  const gap = options.gap ?? 8;
  const margin = options.margin ?? 10;
  const minHeight = options.minHeight ?? 140;

  const spaceRight = viewport.width - anchor.right - gap - margin;
  const spaceLeft = anchor.left - gap - margin;
  const placeRight = spaceRight >= size.width || spaceRight >= spaceLeft;

  const rawLeft = placeRight ? anchor.right + gap : anchor.left - gap - size.width;
  const left = Math.min(Math.max(rawLeft, margin), Math.max(margin, viewport.width - margin - size.width));

  const maxHeight = Math.max(minHeight, viewport.height - 2 * margin);
  const height = Math.min(size.height, maxHeight);
  const top = Math.min(Math.max(anchor.top, margin), Math.max(margin, viewport.height - margin - height));

  return { left: `${left}px`, top: `${top}px`, maxHeight: `${maxHeight}px` };
}

/** Absolute offsets for a menu, plus which side of the anchor it landed on. */
export interface MenuPlacement {
  left: string;
  top: string;
  maxHeight: string;
  side: 'right' | 'left';
}

/**
 * Places a menu at a click point or against a trigger. A right-click menu and
 * a submenu flyout want the same three decisions — open towards the free
 * space, never leave the viewport, and shrink rather than overflow — so both
 * go through here instead of each re-deriving them from `clientX`/`clientY`.
 */
export function placeMenu(
  anchor: AnchorBox,
  viewport: ViewportSize,
  size: PopoverSize,
  options: PlacementOptions & { align?: 'start' | 'stretch' } = {},
): MenuPlacement {
  const gap = options.gap ?? 0;
  const margin = options.margin ?? 8;
  const minHeight = options.minHeight ?? 120;

  const spaceRight = viewport.width - anchor.right - gap - margin;
  const openRight = spaceRight >= size.width || spaceRight >= anchor.left - gap - margin;
  const rawLeft = openRight ? anchor.right + gap : anchor.left - gap - size.width;
  const left = Math.min(
    Math.max(rawLeft, margin),
    Math.max(margin, viewport.width - margin - size.width),
  );

  const spaceBelow = viewport.height - anchor.top - margin;
  const openDown = spaceBelow >= size.height || spaceBelow >= anchor.bottom - margin;
  const maxHeight = Math.max(
    minHeight,
    openDown ? spaceBelow : anchor.bottom - margin,
  );
  const height = Math.min(size.height, maxHeight);
  const rawTop = openDown ? anchor.top : anchor.bottom - height;
  const top = Math.min(
    Math.max(rawTop, margin),
    Math.max(margin, viewport.height - margin - height),
  );

  return {
    left: `${Math.round(left)}px`,
    top: `${Math.round(top)}px`,
    maxHeight: `${Math.round(maxHeight)}px`,
    side: openRight ? 'right' : 'left',
  };
}
