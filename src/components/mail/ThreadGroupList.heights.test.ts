// @vitest-environment jsdom
import { shallowMount } from '@vue/test-utils';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import ThreadGroupList from './ThreadGroupList.vue';

/*
 * ARCH-11. The virtualizer adds row heights up in TypeScript; the markup has to
 * render exactly those numbers. AGENTS.md names the coupling as silently
 * breaking — a wrapper left at 92px while the constant moves to 96 drifts four
 * pixels a row, so after 25 rows a click lands on the wrong thread, and
 * nothing goes red.
 *
 * The heights are bound as custom properties now, so the second copy is gone.
 * These two checks keep it gone: no pixel literal creeps back in, and no
 * wrapper references a property nobody defines (which renders at auto height,
 * equally silent).
 */

// Not `import.meta.url`: under jsdom that is an http URL, and `readFileSync`
// wants a path. Vitest runs from the repository root.
const SOURCE = readFileSync(
  join(process.cwd(), 'src/components/mail/ThreadGroupList.vue'),
  'utf8',
);

/**
 * Pixel height literals in classes, excluding the ones that are not row
 * geometry. `h-[18px]` is the unread badge — it is not summed by anything.
 */
const ALLOWED_LITERALS = new Set(['18']);

export function heightLiterals(source: string): string[] {
  return [...source.matchAll(/(?<![-\w])h-\[(\d+)px\]/g)]
    .map((match) => match[1]!)
    .filter((pixels) => !ALLOWED_LITERALS.has(pixels));
}

/** Custom property names the `rowHeights` object defines. */
function boundProperties(source: string): Set<string> {
  const block = source.match(/const rowHeights = computed\(\(\) => \(\{([\s\S]*?)\}\)\);/);
  if (!block) throw new Error('no rowHeights binding in ThreadGroupList.vue');
  return new Set([...block[1]!.matchAll(/'(--[\w-]+)'/g)].map((match) => match[1]!));
}

export function unboundProperties(source: string): string[] {
  const bound = boundProperties(source);
  return [...source.matchAll(/h-\[var\((--[\w-]+)\)\]/g)]
    .map((match) => match[1]!)
    .filter((name) => !bound.has(name));
}

describe('thread list row heights', () => {
  it('keeps every row height in one place', () => {
    expect(heightLiterals(SOURCE)).toEqual([]);
    expect(unboundProperties(SOURCE)).toEqual([]);
  });

  /** The drift the issue describes: the constant moved, the class did not. */
  it('reports a wrapper that still carries a pixel height', () => {
    const fixture = `
      const THREAD_HEIGHT = 96;
      const rowHeights = computed(() => ({ '--thread-row-h': \`\${THREAD_HEIGHT}px\` }));
      <div class="thread-list-row flex h-[92px] items-center px-2" />
    `;

    expect(heightLiterals(fixture)).toEqual(['92']);
  });

  it('reports a wrapper referencing a property nobody defines', () => {
    const fixture = `
      const rowHeights = computed(() => ({ '--thread-row-h': '92px' }));
      <div class="thread-list-row flex h-[var(--row-h)] items-center px-2" />
    `;

    expect(unboundProperties(fixture)).toEqual(['--row-h']);
  });

  it('lets the badge height through', () => {
    expect(heightLiterals('<span class="inline-flex h-[18px] min-w-[18px]" />')).toEqual([]);
  });

  function mountList() {
    return shallowMount(ThreadGroupList, {
      props: {
        items: [],
        selectedThreadIds: [],
        senderReviewSetupComplete: true,
        bulkBusy: false,
      },
    });
  }

  function scrollerStyle(wrapper: ReturnType<typeof mountList>) {
    return wrapper.get('.thread-list-scroller').attributes('style') ?? '';
  }

  it('puts the heights on the scroller as custom properties', () => {
    const style = scrollerStyle(mountList());
    for (const [name, value] of [
      ['--thread-row-h', '72px'],
      ['--date-header-h', '32px'],
      ['--bucket-h', '72px'],
      ['--folder-card-h', '80px'],
      ['--card-header-h', '40px'],
      ['--review-row-h', '68px'],
      ['--review-banner-h', '44px'],
    ]) {
      expect(style).toContain(`${name}: ${value}`);
    }
  });

  /* tokens.css lifts the type scale at 1020px; three text lines need the taller row there. */
  it('switches to the phone row heights with the phone type scale', () => {
    const original = window.matchMedia;
    window.matchMedia = vi.fn((query: string) => ({
      matches: query === '(max-width: 1020px)',
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })) as unknown as typeof window.matchMedia;
    try {
      const style = scrollerStyle(mountList());
      for (const [name, value] of [
        ['--thread-row-h', '88px'],
        ['--date-header-h', '40px'],
        ['--folder-card-h', '88px'],
        ['--review-row-h', '84px'],
        ['--review-banner-h', '52px'],
      ]) {
        expect(style).toContain(`${name}: ${value}`);
      }
    } finally {
      window.matchMedia = original;
    }
  });
});
