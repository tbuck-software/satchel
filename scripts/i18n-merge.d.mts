export interface SubsetSpec {
  entries: string[];
  extraKeys: string[];
  extraPrefixes: string[];
  backendSources?: string[];
}

export const SUBSETS: Record<string, SubsetSpec>;
export function subsetKeys(name: string, allKeys: Set<string>): string[];
