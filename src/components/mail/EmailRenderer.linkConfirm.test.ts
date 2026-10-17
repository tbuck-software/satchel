// @vitest-environment jsdom

import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/utils/openExternalUrl', () => ({
  openExternalUrl: vi.fn().mockResolvedValue(undefined),
}));

import { openExternalUrl } from '@/utils/openExternalUrl';
import { loadPublicSuffixTable } from '@/utils/externalLinks';
import EmailRenderer from './EmailRenderer.vue';

const openExternalUrlMock = vi.mocked(openExternalUrl);

/**
 * jsdom's `innerWidth` cannot be assigned, so the layout mode comes from a
 * mutable harness instead of a window resize — same approach as the other
 * component tests.
 */
const uiState = vi.hoisted(() => ({
  isSinglePane: false,
  isMobile: false,
  registerOverlay: () => () => {},
}));
vi.mock('@/stores/ui', () => ({
  useUiStore: () => uiState,
}));

const SAFE_TEXT = 'Mehr unter https://example.com/guide im Netz.';
const RISKY_TEXT = 'Zertifikat: https://example.test:8443/path aktiv.';
const HTTP_TEXT = 'Quelle: http://insecure.test/page war verlinkt.';
const SUBDOMAIN_TEXT = 'Artikel: https://news.shop.example.com/article heute.';

function mountRenderer(text: string) {
  return mount(EmailRenderer, {
    props: {
      text,
      senderEmail: 'mara@example.com',
    },
  });
}

/** Pretend the primary input is a finger — phones and tablets alike. */
function useTouchPointer() {
  Object.defineProperty(window, 'matchMedia', {
    configurable: true,
    value: (query: string) => ({
      matches: query === '(hover: none)',
      addEventListener: () => {},
      removeEventListener: () => {},
    }),
  });
}

function firstLink(wrapper: ReturnType<typeof mount>) {
  const link = wrapper.find('a[href^="http"]');
  expect(link.exists()).toBe(true);
  return link;
}

/** Both confirmation surfaces live in a body teleport, not the wrapper tree. */
function bodyButton(label: string): HTMLButtonElement | null {
  return (
    Array.from(document.body.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent?.trim() === label,
    ) ?? null
  );
}

function bodyText(): string {
  return document.body.textContent ?? '';
}

async function confirmWithTrust() {
  const toggle = document.body.querySelector<HTMLInputElement>('input[type="checkbox"]');
  expect(toggle).not.toBeNull();
  toggle!.click();
  await flushPromises();
  (bodyButton('Open') as HTMLButtonElement).click();
  await flushPromises();
}

describe('EmailRenderer external link handling', () => {
  beforeAll(() => loadPublicSuffixTable());

  beforeEach(() => {
    setActivePinia(createPinia());
    window.localStorage.clear();
    uiState.isSinglePane = false;
    uiState.isMobile = false;
    openExternalUrlMock.mockClear();
    delete (window as Partial<typeof window>).matchMedia;
  });

  it('opens a safe link directly on wide screens', async () => {
    const wrapper = mountRenderer(SAFE_TEXT);
    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(openExternalUrlMock).toHaveBeenCalledTimes(1);
    expect(openExternalUrlMock).toHaveBeenCalledWith('https://example.com/guide');
    // Neither surface asks: no dialog, no sheet.
    expect(document.querySelector('.link-dialog')).toBeNull();
    expect(document.querySelector('.sheet-panel')).toBeNull();
    wrapper.unmount();
  });

  it('removes the hover preview at activation instead of keeping it on screen', async () => {
    const wrapper = mountRenderer(SAFE_TEXT);
    const link = firstLink(wrapper);

    await link.trigger('mouseenter');
    expect(document.querySelector('.link-preview-card')).not.toBeNull();

    // A touch tap fires hover but never mouseout; activation must still
    // take the preview away.
    await link.trigger('click');
    await flushPromises();

    expect(document.querySelector('.link-preview-card')).toBeNull();
    wrapper.unmount();
  });

  it('shows no hover preview at all where hover does not exist', async () => {
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: (query: string) => ({
        matches: query !== '(hover: hover)',
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    });
    const wrapper = mountRenderer(SAFE_TEXT);

    await firstLink(wrapper).trigger('mouseenter');

    expect(document.querySelector('.link-preview-card')).toBeNull();
    wrapper.unmount();
  });

  it('confirms risky links on wide screens before opening them', async () => {
    const wrapper = mountRenderer(RISKY_TEXT);
    await firstLink(wrapper).trigger('click');

    expect(openExternalUrlMock).not.toHaveBeenCalled();
    const dialog = document.querySelector('.link-dialog');
    expect(dialog).not.toBeNull();
    // The destination is named, and the port flags why this interrupts.
    expect(dialog?.textContent).toContain('example.test');
    expect(dialog?.textContent).toContain('unusual network port');

    (bodyButton('Open anyway') as HTMLButtonElement).click();
    await flushPromises();

    expect(openExternalUrlMock).toHaveBeenCalledTimes(1);
    expect(document.querySelector('.link-dialog')).toBeNull();
    wrapper.unmount();
  });

  it('asks once per untrusted domain through the bottom sheet on phones', async () => {
    useTouchPointer();
    const wrapper = mountRenderer(SAFE_TEXT);
    const link = firstLink(wrapper);

    // Even a structurally safe destination waits for the confirmation.
    await link.trigger('click');
    await flushPromises();
    expect(openExternalUrlMock).not.toHaveBeenCalled();
    expect(document.querySelector('.sheet-panel')).not.toBeNull();
    expect(bodyText()).toContain('Open external link?');

    (bodyButton('Cancel') as HTMLButtonElement).click();
    await flushPromises();
    expect(openExternalUrlMock).not.toHaveBeenCalled();
    expect(document.querySelector('.sheet-panel')).toBeNull();

    // Trusting opens right away and remembers the registrable domain …
    await link.trigger('click');
    await confirmWithTrust();

    expect(openExternalUrlMock).toHaveBeenCalledTimes(1);
    expect(window.localStorage.getItem('satchel.trustedLinkHosts')).toBe('["example.com"]');

    // … and covers sibling subdomains without asking again.
    window.localStorage.setItem(
      'satchel.trustedLinkHosts',
      JSON.stringify(['other.example.com']),
    );
    setActivePinia(createPinia());
    const nextWrapper = mountRenderer(SAFE_TEXT);
    await firstLink(nextWrapper).trigger('click');
    await flushPromises();

    expect(openExternalUrlMock).toHaveBeenCalledTimes(2);
    expect(document.querySelector('.sheet-panel')).toBeNull();
    wrapper.unmount();
    nextWrapper.unmount();
  });

  it('keeps the warning tone in the sheet for risky destinations only', async () => {
    useTouchPointer();
    const wrapper = mountRenderer(RISKY_TEXT);

    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(document.querySelector('.sheet-panel')).not.toBeNull();
    expect(bodyText()).toContain('Check the link before opening');
    expect(bodyButton('Open anyway')).not.toBeNull();
    expect(openExternalUrlMock).not.toHaveBeenCalled();
    wrapper.unmount();
  });

  it('keeps plain http calm while still naming the risk', async () => {
    useTouchPointer();
    const wrapper = mountRenderer(HTTP_TEXT);

    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(document.querySelector('.sheet-panel')).not.toBeNull();
    expect(bodyText()).toContain('Open external link?');
    expect(bodyText()).toContain('not encrypted');
    wrapper.unmount();
  });

  // #125: trust used to override every warning, and was offered on them too.
  it('warns about a risky link even on a trusted domain, and offers no trust there', async () => {
    window.localStorage.setItem('satchel.trustedLinkHosts', JSON.stringify(['example.test']));
    useTouchPointer();
    const wrapper = mountRenderer(RISKY_TEXT);

    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(openExternalUrlMock).not.toHaveBeenCalled();
    expect(bodyText()).toContain('Check the link before opening');
    expect(document.body.querySelector('input[type="checkbox"]')).toBeNull();
    wrapper.unmount();
  });

  it('names the domain that trusting stores, not the subdomain at hand', async () => {
    useTouchPointer();
    const wrapper = mountRenderer(SUBDOMAIN_TEXT);

    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(bodyText()).toContain('Open links to example.com and its subdomains');
    await confirmWithTrust();
    expect(window.localStorage.getItem('satchel.trustedLinkHosts')).toBe('["example.com"]');
    wrapper.unmount();
  });

  it('never shows the bottom sheet in a narrow desktop window', async () => {
    // Single-pane layout (the window is narrow), but the input is a pointer.
    uiState.isSinglePane = true;
    const wrapper = mountRenderer(SAFE_TEXT);

    await firstLink(wrapper).trigger('click');
    await flushPromises();

    expect(document.querySelector('.link-dialog')).not.toBeNull();
    expect(document.querySelector('.sheet-panel')).toBeNull();
    wrapper.unmount();
  });
});
