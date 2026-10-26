// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const harness = vi.hoisted(() => ({
  saveProfile: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  unregister: vi.fn(),
  registerOverlay: vi.fn(),
}));
vi.mock('@/stores/contacts', () => ({ useContactsStore: () => ({ saveProfile: harness.saveProfile }) }));
vi.mock('@/stores/ui', () => ({ useUiStore: () => ({ registerOverlay: harness.registerOverlay }) }));
vi.mock('@/composables/useToast', () => ({ useToast: () => ({ success: harness.success, error: harness.error }) }));
import AddContactDialog from './AddContactDialog.vue';

function render() {
  return mount(AddContactDialog, { global: { stubs: {
    ModalShell: { template: '<section><slot /></section>' },
    PaneHeader: true,
  } } });
}

beforeEach(() => {
  vi.clearAllMocks();
  harness.registerOverlay.mockReturnValue(harness.unregister);
  harness.saveProfile.mockResolvedValue({});
});

describe('AddContactDialog', () => {
  it('saves through the contact store and closes only after success', async () => {
    const wrapper = render();
    await wrapper.get('input[type=email]').setValue('person@example.com');
    await wrapper.get('input[autocomplete=name]').setValue('Person');
    await wrapper.get('form').trigger('submit');
    await flushPromises();
    expect(harness.saveProfile).toHaveBeenCalledWith({ email: 'person@example.com', displayName: 'Person', notes: '' });
    expect(wrapper.emitted('dismiss')).toHaveLength(1);
    expect(harness.success).toHaveBeenCalledWith('Contact saved.');
    wrapper.unmount();
    expect(harness.unregister).toHaveBeenCalledOnce();
  });

  it('keeps entered data and reports a failed save', async () => {
    harness.saveProfile.mockRejectedValue(new Error('database is locked'));
    const wrapper = render();
    await wrapper.get('input[type=email]').setValue('person@example.com');
    await wrapper.get('form').trigger('submit');
    await flushPromises();
    expect(wrapper.emitted('dismiss')).toBeUndefined();
    expect(harness.error).toHaveBeenCalledWith(expect.stringContaining('The contact could not be saved.'));
    expect(wrapper.get('input').element.value).toBe('person@example.com');
    expect(wrapper.get('button[type=submit]').attributes('disabled')).toBeUndefined();
    wrapper.unmount();
  });
});
