/**
 * Whether the shell has a tray or a launch agent at all.
 *
 * Asked rather than guessed: the window width says nothing about the platform,
 * and both switches on that page call commands a phone refuses.
 */
export interface DesktopBackgroundCapabilities {
  tray: boolean;
  autostart: boolean;
}

export function supportsDesktopBackgroundControls(
  capabilities: DesktopBackgroundCapabilities,
): boolean {
  return capabilities.tray || capabilities.autostart;
}
