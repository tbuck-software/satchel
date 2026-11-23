import { Ban, Clock } from 'lucide';
import { describe, expect, it } from 'vitest';
import { inlineLucideImports } from './vite-lucide-inline.mjs';

const fail = (/** @type {string} */ message) => {
  throw new Error(message);
};

describe('inlineLucideImports', () => {
  it('turns each imported icon into a local constant with the same data', () => {
    const output = inlineLucideImports(
      "import { Ban, Clock as Later } from 'lucide';\nexport const icons = [Ban, Later];",
      fail,
    );

    expect(output).not.toContain('lucide');
    const icons = new Function(`${output?.replace('export ', '')}; return icons;`)();
    expect(icons).toEqual([Ban, Clock]);
  });

  it('drops type-only specifiers and leaves other imports alone', () => {
    const output = inlineLucideImports(
      'import { computed } from "vue";\nimport { Ban, type IconNode } from "lucide";',
      fail,
    );

    expect(output).toBe(`import { computed } from "vue";\nconst Ban = ${JSON.stringify(Ban)};`);
  });

  it('returns null for a module without a Lucide import', () => {
    expect(inlineLucideImports("import { ref } from 'vue';", fail)).toBeNull();
  });

  it('fails the build on a name Lucide does not export', () => {
    expect(() => inlineLucideImports("import { NoSuchIcon } from 'lucide';", fail))
      .toThrow('lucide has no icon named NoSuchIcon');
  });
});
