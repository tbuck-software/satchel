import { clearanceAbove, toastObstacleRects } from './useToastClearance';

export interface ToastPlacement {
  clearance: number;
  reserved: boolean;
  availableWidth: number | null;
  remeasure?: boolean;
}

/** Loaded with the first receipt, so narrow viewers pay no startup cost. */
export function measureToastPlacement(stack: HTMLElement, previous: ToastPlacement, gap: number): ToastPlacement {
  const rect = stack.getBoundingClientRect();
  const composer = document.querySelector<HTMLElement>('.composer-window');
  const leftRoom = composer ? composer.getBoundingClientRect().left - rect.left - gap : 440;
  const availableWidth = leftRoom >= 240 && leftRoom < 440 ? leftRoom : null;
  // Wrap into the free desktop corner before taking space from the canvas.
  if (previous.availableWidth !== availableWidth) {
    return { clearance: 0, reserved: false, availableWidth, remeasure: true };
  }
  const resting = { top: rect.top + previous.clearance, bottom: rect.bottom + previous.clearance,
    left: rect.left, right: rect.right };
  const clearance = clearanceAbove(resting, toastObstacleRects(stack), gap);
  const reserved = document.querySelector('.composer-window--maximized') !== null
    || (previous.reserved && composer !== null && availableWidth === null && leftRoom < 240)
    || (composer !== null && clearance > Math.max(0, resting.top - gap));
  return { clearance: reserved ? 0 : clearance, reserved, availableWidth };
}
