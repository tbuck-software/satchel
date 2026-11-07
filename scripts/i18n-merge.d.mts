export interface SubsetSpec {
  entries: string[];
  extraKeys: string[];
  extraPrefixes: string[];
  backendSources?: string[];
}

type Bundle = Record<string, unknown>;

export const SUBSETS: Record<string, SubsetSpec>;
export function subsetKeys(name: string, allKeys: Set<string>): string[];
export function buildBundles(): { bundles: Record<'en' | 'de', Bundle>; conflicts: string[] };
export function keyPaths(value: unknown, trail?: string[]): string[];
export function pick(bundle: Bundle, keys: Iterable<string>): Bundle;
export function sortDeep<T>(value: T): T;
