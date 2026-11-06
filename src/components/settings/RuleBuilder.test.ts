// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, expect, it, vi } from 'vitest';
import RuleBuilder from './RuleBuilder.vue';

const backend = vi.hoisted(() => ({
  backendListRules: vi.fn(), backendUpsertRule: vi.fn(), backendDeleteRule: vi.fn(),
  backendRunRulesForThread: vi.fn(), backendRunRulesForThreads: vi.fn(),
  backendGetServerRulesOverview: vi.fn(),
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

it('marks each active rule as running on the server or staying local, with the reason', async () => {
  backend.backendListRules.mockResolvedValue([
    { id: 'r-server', name: 'Newsletters', priority: 0, is_active: true, trigger_config: { from_contains: 'news@' }, actions: ['mark_read'] },
    { id: 'r-local', name: 'Invoices', priority: 0, is_active: true, trigger_config: { subject_contains: 'invoice' }, actions: ['set_state:done'] },
    { id: 'r-paused', name: 'Paused', priority: 0, is_active: false, trigger_config: { from_contains: 'x' }, actions: ['mark_read'] },
  ]);
  backend.backendGetServerRulesOverview.mockResolvedValue({
    accounts: [{ account_id: 'acc', display_name: 'Work', email: 'me@example.com', enabled: true, server_active: true }],
    rules: [
      { rule_id: 'r-server', server_account_ids: ['acc'], local_reason: null, pending: false },
      { rule_id: 'r-local', server_account_ids: [], local_reason: { kind: 'action', detail: 'set_state' }, pending: false },
    ],
  });
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  const badges = wrapper.findAll('[data-testid="rule-placement"]');
  expect(badges.map((badge) => badge.text())).toEqual(['Also runs on server', 'Local only']);
  expect(badges[0]!.attributes('title')).toContain('Also runs on the server for Work');
  expect(badges[0]!.attributes('title')).toContain('whole conversation');
  expect(badges[1]!.attributes('title')).toBe('“Set status” exists only in Satchel.');
});

it('shows no placement badges while no account has server rules on', async () => {
  backend.backendListRules.mockResolvedValue([
    { id: 'r', name: 'Any', priority: 0, is_active: true, trigger_config: { from_contains: 'x' }, actions: ['mark_read'] },
  ]);
  backend.backendGetServerRulesOverview.mockResolvedValue({ accounts: [{ account_id: 'acc', enabled: false }], rules: [] });
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  expect(wrapper.find('[data-testid="rule-placement"]').exists()).toBe(false);
});
