// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import { mount, type VueWrapper } from '@vue/test-utils';
import { useUiStore } from '@/stores/ui';
import MessageRecipientDetails from './MessageRecipientDetails.vue';

let wrapper: VueWrapper | null = null;

beforeEach(() => {
  setActivePinia(createPinia());
  useUiStore().isSinglePane = true;
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = '';
});

describe('message recipient details', () => {
  it('summarizes recipients and reveals the complete address list', async () => {
    wrapper = mount(MessageRecipientDetails, {
      attachTo: document.body,
      props: {
        from: { name: 'Stefan Bente', email: 'stefan@example.com' },
        subject: 'Modulbeschreibungen für die PO-Version 3',
        timestamp: '2026-08-13T16:40:00Z',
        replyTo: 'sekretariat@example.com',
        to: [
          { name: 'Professoren', email: 'prof@example.com' },
          { name: 'Inf MIT', email: 'inf_mit@example.com' },
          { name: 'Inf Alle', email: 'inf_alle@example.com' },
        ],
        cc: [{ name: 'Prüfungsamt', email: 'pa@example.com' }],
      },
    });

    const trigger = wrapper.get('.recipient-summary');
    expect(trigger.text()).toBe('To: Professoren +2');

    await trigger.trigger('click');

    const details = document.body.querySelector('.recipient-details');
    expect(details?.textContent).not.toContain('Message details');
    expect(details?.textContent).toContain('Modulbeschreibungen für die PO-Version 3');
    expect(details?.querySelector('time')?.getAttribute('datetime')).toBe('2026-08-13T16:40:00Z');
    expect(details?.textContent).toContain('From');
    expect(details?.textContent).toContain('Stefan Bente');
    expect(details?.textContent).toContain('Inf Alle');
    expect(details?.textContent).toContain('Prüfungsamt');
    expect(details?.textContent).toContain('Reply-To');
    expect(details?.textContent).toContain('sekretariat@example.com');
    expect(document.body.querySelector('.sheet-grip')).not.toBeNull();
  });

  it('keeps avatar initials white while address text stays muted', async () => {
    wrapper = mount(MessageRecipientDetails, {
      attachTo: document.body,
      props: {
        from: { name: 'Stefan Bente', email: 'stefan@example.com' },
        to: [{ name: 'Professoren', email: 'prof@example.com' }],
        subject: 'Betreff',
        timestamp: '2026-08-13T16:40:00Z',
      },
    });
    await wrapper.get('.recipient-summary').trigger('click');

    const avatarInitials = document.body.querySelector('.recipient-person .avatar__initials');
    const addressNodes = [...document.body.querySelectorAll('.recipient-person > div > span')];
    expect(avatarInitials).not.toBeNull();
    expect(addressNodes).not.toContain(avatarInitials);
    expect(addressNodes.map((node) => node.textContent)).toContain('stefan@example.com');
  });

  it('shows every available sender-authentication verdict without overstating identity', async () => {
    wrapper = mount(MessageRecipientDetails, {
      attachTo: document.body,
      props: {
        from: { name: 'GitLab', email: 'gitlab@code.example' },
        to: [{ name: 'Alex', email: 'alex@example.com' }],
        subject: 'Pipeline status',
        timestamp: '2026-09-03T10:00:03Z',
        authSpf: null,
        authDkim: 'pass',
        authDmarc: 'fail',
      },
    });

    expect(wrapper.get('.recipient-summary').text()).toBe('To: Alex');
    await wrapper.get('.recipient-summary').trigger('click');

    const authentication = document.body.querySelector('.security-row--authentication');
    expect(authentication?.classList.contains('security-row--fail')).toBe(true);
    expect(authentication?.textContent).toContain('Authentication');
    expect(authentication?.textContent).toContain('DKIM');
    expect(authentication?.textContent).toContain('Passed');
    expect(authentication?.textContent).toContain('SPF');
    expect(authentication?.textContent).toContain('Not available');
    expect(authentication?.textContent).toContain('DMARC');
    expect(authentication?.textContent).toContain('Failed');
    expect(authentication?.textContent).not.toContain('Sender verified');
  });

  it('shows a verified S/MIME signer in the sender details', async () => {
    wrapper = mount(MessageRecipientDetails, {
      attachTo: document.body,
      props: {
        from: { name: 'GitLab', email: 'gitlab@code.example' },
        to: [{ name: 'Alex', email: 'alex@example.com' }],
        subject: 'Pipeline status',
        timestamp: '2026-09-03T10:00:03Z',
        smimeState: 'signature_valid',
        smimeSignerCn: 'GitLab Service',
        smimeSignerEmail: 'gitlab@code.example',
      },
    });

    (wrapper.vm as unknown as { openDetails: () => void }).openDetails();
    await wrapper.vm.$nextTick();

    const details = document.body.querySelector('.security-row--smime');
    expect(document.body.querySelector('.recipient-details')?.textContent).toContain('Security');
    expect(details?.classList.contains('security-row--success')).toBe(true);
    expect(details?.textContent).toContain('Digital signature valid');
    expect(details?.textContent).toContain('Signed by');
    expect(details?.textContent).toContain('GitLab Service');
    expect(details?.textContent).toContain('gitlab@code.example');
  });
});
