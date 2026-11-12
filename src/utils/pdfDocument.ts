import { getDocument, GlobalWorkerOptions } from 'pdfjs-dist';
import workerUrl from 'pdfjs-dist/build/pdf.worker.min.mjs?url';

GlobalWorkerOptions.workerSrc = workerUrl;

// Vite emits these beside the app. PDF rendering must also work offline,
// including documents using embedded fonts, CJK character maps and JPEG2000.
const assets = import.meta.glob<string>([
  '/node_modules/pdfjs-dist/cmaps/*.bcmap',
  '/node_modules/pdfjs-dist/standard_fonts/*.{ttf,pfb}',
  '/node_modules/pdfjs-dist/wasm/*.{wasm,js}',
], { query: '?url&no-inline', import: 'default', eager: true });
const urls = new Map(Object.entries(assets).map(([path, url]) => [path.split('/').pop()!, url]));

class LocalPdfDataFactory {
  async fetch({ filename }: { filename: string }): Promise<Uint8Array> {
    const url = urls.get(filename);
    if (!url) throw new Error(`Missing bundled PDF resource: ${filename}`);
    const response = await fetch(url);
    if (!response.ok) throw new Error(`Could not load PDF resource: ${filename}`);
    return new Uint8Array(await response.arrayBuffer());
  }
}

export function loadPdfDocument(bytes: Uint8Array) {
  return getDocument({
    // PDF.js transfers its input to the worker. Keep the attachment available
    // for Save as and for a replacement preview.
    data: bytes.slice(),
    useWorkerFetch: false,
    useSystemFonts: false,
    BinaryDataFactory: LocalPdfDataFactory,
  });
}
