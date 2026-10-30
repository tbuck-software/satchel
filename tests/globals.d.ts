/**
 * What the render-fidelity page hangs on `window` for a test to drive it.
 *
 * The app declares the same property in `RenderFidelityApp.vue`; that file is
 * not in this project's `include`, so the hook has to be named again here. The
 * payload stays structural on purpose — a test that builds one from literals
 * should not have to import a component's private interface.
 */
interface RenderFidelityPayload {
  trustedDmarcPass?: boolean;
  id: string;
  label: string;
  subject: string;
  fromAddress: string;
  fromName?: string;
  date?: string;
  bodyHtml?: string;
  bodyText?: string;
  normalizeQuotes?: boolean;
  attachments: unknown[];
  meta: string[];
}

interface Window {
  __mountSatchelResendEditor?: (sourceHtml: string) => void;
  __setSatchelRenderFidelityPayload?: (payload: RenderFidelityPayload) => Promise<void>;
}
