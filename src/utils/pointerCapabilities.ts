/**
 * Whether the primary pointer can hover — mouse and trackpad, including on
 * touch-capable hardware. Layout width says nothing about this: a halved
 * desktop window is narrow yet stays a pointer device, while a wide tablet
 * stays touch. Surfaces that exist *because* a finger is the input (bottom
 * sheets, drag-to-dismiss) belong to the touch side of this line; pointer
 * affordances like hover previews to the other.
 */
export function hasHoverCapablePointer(): boolean {
  if (typeof window === 'undefined') return true;
  return window.matchMedia?.('(hover: hover)').matches !== false;
}
