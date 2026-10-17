import { defineConfig, loadEnv } from 'vite';
import tailwindcss from '@tailwindcss/vite';
import vue from '@vitejs/plugin-vue';
import VueI18nPlugin from '@intlify/unplugin-vue-i18n/vite';
import { fileURLToPath, URL } from 'node:url';

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
    plugins: [tailwindcss(), vue(), ...(command === 'build' ? [VueI18nPlugin({
      include: [fileURLToPath(new URL('./src/i18n/*.json', import.meta.url))],
      runtimeOnly: true,
      strictMessage: false,
      dropMessageCompiler: true,
    })] : [])],
    build: {
      manifest: true,
      rollupOptions: {
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
          manualChunks(id) {
            if (!id.includes('node_modules')) return undefined;
            if (id.includes('@tiptap')) return 'vendor-editor';
            if (id.includes('@tauri-apps')) return 'vendor-tauri';
            if (
              id.includes('/node_modules/vue/')
              || id.includes('/node_modules/@vue/')
              || id.includes('/node_modules/pinia/')
              || id.includes('/node_modules/vue-router/')
            ) {
              return 'vendor-vue';
            }
            // Keep independently loaded feature dependencies independent.
            if (id.includes('/node_modules/date-fns/')) return 'vendor-date';
            // Public-suffix tables. Loaded on demand by utils/externalLinks, so
            // this must stay a chunk of its own or it rejoins an entry.
            if (id.includes('/node_modules/tldts')) return 'vendor-psl';
            if (id.includes('/node_modules/marked/')) return 'vendor-markdown';
            if (id.includes('/node_modules/@vueuse/')) return 'vendor-vueuse';
            // The viewer and tray load i18n too; keeping it out of vendor-vue
            // makes the cost of translating those windows visible in the build.
            if (id.includes('/node_modules/vue-i18n/') || id.includes('/node_modules/@intlify/')) {
              return 'vendor-i18n';
            }
            return undefined;
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
      exclude: ['**/node_modules/**', '**/dist/**', '**/.claude/**', '**/target/**'],
      setupFiles: ['./src/test/setup.ts'],
    },
  };
});
