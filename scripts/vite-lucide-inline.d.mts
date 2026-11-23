import type { Plugin } from 'vite';

export function inlineLucideImports(code: string, fail: (message: string) => never): string | null;
export function lucideInline(): Plugin;
