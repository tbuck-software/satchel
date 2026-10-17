/**
 * Subscribing to a Rust event, without a component importing `@tauri-apps`.
 *
 * The disposer is returned rather than the listener handle so a caller cannot
 * hold on to anything else — a listener that outlives its component is the
 * failure this indirection is meant to make hard.
 */
export async function listenToBackend<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<T>(event, (message) => handler(message.payload));
}
