// @ts-check
import { readFileSync } from 'node:fs';
import { build } from 'vite';

// A standalone Node installer needs neither a checkout nor frontend dependencies.
await build({
  configFile: false,
  build: {
    ssr: 'scripts/install-release.mjs',
    target: 'node22',
    outDir: 'output/installer',
    emptyOutDir: true,
    minify: false,
    rollupOptions: {
      output: {
        entryFileNames: 'install-satchel.mjs',
        inlineDynamicImports: true,
        banner: `/*\n${readFileSync('LICENSE', 'utf8').trim()}\n*/`,
      },
    },
  },
});
