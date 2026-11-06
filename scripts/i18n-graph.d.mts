export const repoRoot: string;
export const DYNAMIC_IMPORT_RE: RegExp;

export interface KeyIndex {
  has(key: string): boolean;
  expand(literal: string, open: boolean): string[];
}

export function resolveImport(spec: string, fromFile: string): string | null;
export function moduleGraph(entries: string[]): string[];
export function messageKeys(files: string[]): Set<string>;
export function moduleEdges(file: string): { static: string[]; dynamic: string[] };
export function staticClosure(files: string[]): Set<string>;
export function clearGraphCache(): void;
export function keyIndex(keys: Iterable<string>): KeyIndex;
export function keyMentions(file: string, index: KeyIndex): Set<string>;
export function mentionsInSource(source: string, index: KeyIndex): Set<string>;
