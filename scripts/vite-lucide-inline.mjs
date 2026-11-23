/**
 * Writes the Lucide icons a module imports into that module as constants
 * (ADR 0021).
 *
 * `import { Clock } from 'lucide'` is a module of its own per icon. An icon
 * shared by startup code and a lazy view becomes a chunk of a few dozen bytes,
 * and those shared modules made the bundler split the main entry differently:
 * the window's initial JavaScript grew by 4.5 KiB gzip of chunk glue, more
 * than the icons themselves. A copy per importing module keeps the chunk graph
 * as it was; an icon used by several lazy views is repeated in each, which
 * costs a few bytes once per view.
 *
 * Builds only. The dev server and Vitest import `lucide` as it is.
 */
import * as lucide from 'lucide';

const LUCIDE_IMPORT = /import\s*\{([^}]*)\}\s*from\s*(['"])lucide\2;?/g;

/** @param {string} code @param {(message: string) => never} fail */
export function inlineLucideImports(code, fail) {
  if (!code.includes('lucide')) return null;
  let changed = false;
  const output = code.replace(LUCIDE_IMPORT, (_statement, /** @type {string} */ specifiers) => {
    changed = true;
    return specifiers
      .split(',')
      .map((specifier) => specifier.trim())
      .filter((specifier) => specifier && !specifier.startsWith('type '))
      .map((specifier) => {
        const [imported, local = imported] = specifier.split(/\s+as\s+/);
        const icon = /** @type {Record<string, unknown>} */ (lucide)[imported];
        if (!Array.isArray(icon)) fail(`lucide has no icon named ${imported}`);
        return `const ${local} = ${JSON.stringify(icon)};`;
      })
      .join('\n');
  });
  return changed ? output : null;
}

/** @returns {import('vite').Plugin} */
export function lucideInline() {
  return {
    name: 'satchel:lucide-inline',
    apply: 'build',
    // After the Vue and TypeScript transforms, so type-only imports are gone
    // and an SFC's imports are plain JavaScript.
    enforce: 'post',
    transform(code, id) {
      if (id.includes('/node_modules/') || !/\.(?:vue|ts|js|mjs)(?:$|\?)/.test(id)) return null;
      const output = inlineLucideImports(code, (message) => this.error(`${message} (${id})`));
      return output === null ? null : { code: output, map: null };
    },
  };
}
