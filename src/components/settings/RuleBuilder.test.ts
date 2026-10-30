// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, expect, it, vi } from 'vitest';
import RuleBuilder from './RuleBuilder.vue';

const backend = vi.hoisted(() => ({
  backendListRules: vi.fn(), backendUpsertRule: vi.fn(), backendDeleteRule: vi.fn(),
  backendRunRulesForThread: vi.fn(), backendRunRulesForThreads: vi.fn(),
}));
vi.mock('@/services/backend', () => backend);
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({ visibleThreads: [], activeThreadId: '' }) }));
beforeEach(() => { vi.resetAllMocks(); backend.backendListRules.mockResolvedValue([]); });

it('offers each condition once, stops adding when full, and saves every visible value', async () => {
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  await wrapper.findAll('button').find((b) => b.text() === 'New rule')!.trigger('click');
  await wrapper.get('form > label input').setValue('All fields');
  const add = () => wrapper.findAll('button').find((b) => b.text() === 'Add condition')!;
  for (let i = 0; i < 3; i++) await add().trigger('click');
  expect(add().attributes('disabled')).toBeDefined();
  const rows = wrapper.findAll('fieldset')[0]!.findAll('.flow-row');
  expect(rows).toHaveLength(4);
  for (const [index, row] of rows.entries()) {
    const options = row.findAll('option');
    expect(options.filter((o) => o.attributes('disabled') === undefined)).toHaveLength(1);
    await row.get('input').setValue(`value ${index}`);
  }
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(backend.backendUpsertRule).toHaveBeenCalledWith(expect.objectContaining({
    trigger_config: { match: 'all', from_contains: 'value 0', to_contains: 'value 1', subject_contains: 'value 2', body_contains: 'value 3' },
  }));
});

it('rejects duplicate fields at submit even if a select change bypasses disabled options', async () => {
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  await wrapper.findAll('button').find((b) => b.text() === 'New rule')!.trigger('click');
  await wrapper.get('form > label input').setValue('No overwrite');
  await wrapper.findAll('button').find((b) => b.text() === 'Add condition')!.trigger('click');
  const rows = wrapper.findAll('fieldset')[0]!.findAll('.flow-row');
  await rows[0]!.get('input').setValue('first');
  await rows[1]!.get('input').setValue('second');
  await rows[1]!.get('select').setValue('from_contains');
  await wrapper.get('form').trigger('submit');
  expect(backend.backendUpsertRule).not.toHaveBeenCalled();
  expect(wrapper.get('[role="alert"]').text()).toBe('Use each condition field only once.');
});

it('round-trips supported fields from an existing or imported rule', async () => {
  const config = { match: 'any', from_contains: 'alice', body_contains: 'invoice' };
  backend.backendListRules.mockResolvedValue([{ id: 'r', name: 'Imported', priority: 1, is_active: true, trigger_config: config, actions: ['mark_read'] }]);
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  await wrapper.findAll('button').find((b) => b.text() === 'Edit')!.trigger('click');
  const rows = wrapper.findAll('fieldset')[0]!.findAll('.flow-row');
  expect(rows).toHaveLength(2);
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(backend.backendUpsertRule).toHaveBeenCalledWith(expect.objectContaining({ id: 'r', trigger_config: config }));
});
