import { defineConfig, loadEnv } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import vue from '@vitejs/plugin-vue';
import VueI18nPlugin from '@intlify/unplugin-vue-i18n/vite';
import { fileURLToPath, URL } from 'node:url';
import { i18nChunks } from './scripts/vite-i18n-chunks.mjs';
import { lucideInline } from './scripts/vite-lucide-inline.mjs';

const tauriDevHost = process.env.TAURI_DEV_HOST;

export default defineConfig(({ mode, command }) => {
  const env = loadEnv(mode, process.cwd(), '');
  /*
   * `dev:mock` swaps the command transport, not the stores. Everything above
   * `services/backend/transport` runs the same code it runs in the app; only
   * the far side of `invoke` answers from fixtures. Vitest deliberately does
   * NOT get the swap: unit tests stub the service layer themselves.
   */
  const mockBackend = env.VITE_SATCHEL_MOCK === '1' || env.VITE_SATCHEL_MOCK === 'true';

  return {
    plugins: [tailwindcss(), vue(), lucideInline(), ...(process.env.VITEST ? [] : i18nChunks({ mock: mockBackend })), ...(command === 'build' ? [VueI18nPlugin({
      // `chunks/` is not on disk: `i18nChunks` answers those paths with the
      // keys lazy code needs (ADR 0017).
      include: [
        fileURLToPath(new URL('./src/i18n/*.json', import.meta.url)),
        fileURLToPath(new URL('./src/i18n/chunks/*.json', import.meta.url)),
      ],
      runtimeOnly: true,
      strictMessage: false,
      dropMessageCompiler: true,
    })] : [])],
    build: {
      // Preserve Vite 6's targets: Satchel still supports iOS 15 WKWebView.
      target: ['es2020', 'edge88', 'firefox78', 'chrome87', 'safari14'],
      manifest: true,
      rolldownOptions: {
        // The viewer is a second entry on purpose: an .eml opened from Finder
        // must not pay for the client's router, stores and sync bootstrap before
        // the message appears. The tray briefing follows the same rule: opening
        // a menu-bar panel must not boot a second mail client behind it.
        input: {
          main: fileURLToPath(new URL('./index.html', import.meta.url)),
          viewer: fileURLToPath(new URL('./viewer.html', import.meta.url)),
          tray: fileURLToPath(new URL('./tray.html', import.meta.url)),
        },
        output: {
          codeSplitting: {
            // Do not capture shared dependencies recursively: the editor must
            // stay lazy in the client and absent from the viewer and tray.
            includeDependenciesRecursively: false,
            groups: [{
              debugName: 'feature-vendors',
              name(id) {
                if (!id.includes('node_modules')) return undefined;
                // Tiptap and ProseMirror share initialization-time classes; keep
                // them together so their chunks cannot form an execution cycle.
                if (id.includes('@tiptap') || /\/node_modules\/(?:prosemirror-[^/]+|orderedmap|rope-sequence|w3c-keyname|linkifyjs)\//.test(id)) return 'vendor-editor';
                // Native APIs have different lifetimes: event registration is part
                // of startup, while file dialogs, transfers and extra windows load
                // on use. Let the bundler preserve those dynamic import boundaries.
                if (id.includes('@tauri-apps')) return undefined;
                if (
                  id.includes('/node_modules/vue/')
                  || id.includes('/node_modules/@vue/')
                  || id.includes('/node_modules/pinia/')
                  || id.includes('/node_modules/vue-i18n/')
                  || id.includes('/node_modules/@intlify/')
                ) {
                  return 'vendor-vue';
                }
                if (id.includes('/node_modules/vue-router/')) return 'vendor-router';
                // Keep independently loaded feature dependencies independent.
                if (id.includes('/node_modules/date-fns/')) return 'vendor-date';
                // Public-suffix tables. Loaded on demand by utils/externalLinks, so
                // this must stay a chunk of its own or it rejoins an entry.
                if (id.includes('/node_modules/tldts')) return 'vendor-psl';
                if (id.includes('/node_modules/marked/')) return 'vendor-markdown';
                if (id.includes('/node_modules/@vueuse/')) return 'vendor-vueuse';
                return undefined;
              },
            }],
          },
        },
      },
    },
    resolve: {
      alias: [
        ...(mockBackend
          ? [{
            find: /^@\/services\/backend\/transport$/,
            replacement: fileURLToPath(new URL('./src/services/backend/mock.ts', import.meta.url)),
          }]
          : []),
        { find: '@', replacement: fileURLToPath(new URL('./src', import.meta.url)) },
      ],
    },
    server: {
      host: tauriDevHost || false,
      port: 5173,
      // Rust builds produce thousands of object files. Watching that tree can
      // stall the dev server while Cargo is compiling or linking.
      watch: { ignored: ['**/target/**'] },
    },
    test: {
      // Agent worktrees carry a full source copy; vitest must not walk into them.
      exclude: ['**/node_modules/**', '**/dist/**', '**/dist-mock/**', '**/.claude/**', '**/target/**'],
      setupFiles: ['./src/test/pinLanguage.ts', './src/test/setup.ts'],
    },
  };
});
