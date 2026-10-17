import { perfPersistence } from '@/persistence/perf';

export function perfEnabled(): boolean {
  return perfPersistence.enabled.read();
}

export function perfStart(label: string): (() => void) | null {
  if (!perfEnabled()) {
    return null;
  }

  const startedAt = performance.now();
  console.info(`[perf:start] ${label}`);

  return () => {
    const elapsed = performance.now() - startedAt;
    console.info(`[perf:end] ${label} ${elapsed.toFixed(1)}ms`);
  };
}

export function perfLog(label: string, detail: string): void {
  if (!perfEnabled()) {
    return;
  }

  console.info(`[perf] ${label} ${detail}`);
}
