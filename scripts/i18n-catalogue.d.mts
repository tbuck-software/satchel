type Bundle = Record<string, unknown>;

export function buildBundles(): { bundles: Record<'en' | 'de', Bundle>; conflicts: string[] };
export function keyPaths(value: unknown, trail?: string[]): string[];
export function pick(bundle: Bundle, keys: Iterable<string>): Bundle;
export function sortDeep<T>(value: T): T;
