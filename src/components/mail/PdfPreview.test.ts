// @vitest-environment jsdom
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import PdfPreview from './PdfPreview.vue';

const harness = vi.hoisted(() => ({ load: vi.fn() }));
vi.mock('@/utils/pdfDocument', () => ({ loadPdfDocument: harness.load }));
let wrapper: VueWrapper | null = null;
beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} });
});
afterEach(() => { wrapper?.unmount(); wrapper = null; document.body.innerHTML = ''; vi.unstubAllGlobals(); });

it('keeps the loading indicator until rendering finishes', async () => {
  let rendered!: () => void;
  const cancel = vi.fn();
  const page = {
    getViewport: () => ({ width: 300, height: 400 }),
    render: () => ({ promise: new Promise<void>(resolve => { rendered = resolve; }), cancel }),
    cleanup: vi.fn(),
  };
  const destroy = vi.fn();
  harness.load.mockReturnValue({ promise: Promise.resolve({ numPages: 1, getPage: () => Promise.resolve(page) }), destroy });
  wrapper = mount(PdfPreview, { attachTo: document.body, props: { bytes: new Uint8Array([1]) } });
  await flushPromises();
  expect(wrapper.get('[role="status"]').text()).toBe('Loading attachment…');
  expect(wrapper.get('canvas').isVisible()).toBe(false);
  rendered();
  await flushPromises();
  expect(wrapper.find('[role="status"]').exists()).toBe(false);
  expect(wrapper.get('canvas').isVisible()).toBe(true);
  wrapper.unmount(); wrapper = null;
  expect(destroy).toHaveBeenCalledTimes(1);
  expect(cancel).toHaveBeenCalledTimes(1);
});

it('ignores errors from a replaced document and destroys its worker', async () => {
  let reject!: (reason: Error) => void;
  const destroy = vi.fn();
  harness.load.mockReturnValueOnce({ promise: new Promise((_resolve, fail) => { reject = fail; }), destroy })
    .mockReturnValueOnce({ promise: new Promise(() => {}), destroy: vi.fn() });
  wrapper = mount(PdfPreview, { attachTo: document.body, props: { bytes: new Uint8Array([1]) } });
  await wrapper.setProps({ bytes: new Uint8Array([2]) });
  reject(new Error('Loading aborted'));
  await flushPromises();
  expect(destroy).toHaveBeenCalledTimes(1);
  expect(wrapper.emitted('failed')).toBeUndefined();
  expect(wrapper.get('[role="status"]').isVisible()).toBe(true);
});

it.each(['synchronous', 'asynchronous'])('reports %s document errors', async mode => {
  if (mode === 'synchronous') harness.load.mockImplementationOnce(() => { throw new Error('Empty PDF'); });
  else harness.load.mockReturnValueOnce({ promise: Promise.reject(new Error('Invalid PDF')), destroy: vi.fn() });
  wrapper = mount(PdfPreview, { attachTo: document.body, props: { bytes: new Uint8Array() } });
  await flushPromises();
  expect(wrapper.emitted('failed')).toHaveLength(1);
});
