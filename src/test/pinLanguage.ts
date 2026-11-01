/*
 * Node derives `navigator.language` from the machine's `LANG`, and the `system`
 * preference resolves the app locale from it. On a German machine 69 tests
 * then read German copy where they assert English. This runs before
 * `setup.ts`, whose imports already choose the locale.
 */
for (const [property, value] of [['language', 'en-US'], ['languages', ['en-US', 'en']]] as const) {
  if (typeof navigator !== 'undefined') {
    Object.defineProperty(navigator, property, { configurable: true, get: () => value });
  }
}
