export interface Chunk {
  id: string;
  file: string;
  keys: string[];
}

export interface Split {
  startupKeys: string[];
  startupModules: Set<string>;
  modules: Set<string>;
  chunks: Map<string, Chunk>;
  closures: Map<string, Set<string>>;
  unmentioned: string[];
}

export const MAIN_ENTRY: string;
export const OTHER_ENTRIES: string[];
export const STARTUP: { prefixes: string[]; rustSources: string[] };
export const UNANCHORED_KEYS: Record<string, string[]>;
export const NOT_KEYS: Record<string, string>;
export const FIRST_PAINT: string[];
export const IDLE_PREFETCH: string[];
export function unanchoredReason(argument: string): string | null;
export function moduleKeys(file: string, index: import('./i18n-graph.mjs').KeyIndex): Iterable<string>;
export function chunkId(file: string): string;
export function computeSplit(allKeys: Iterable<string>, options?: { startupRoots?: string[] }): Split;
export function unanchoredKeyCalls(files: Iterable<string>): string[];
