// @vitest-environment jsdom
import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, expect, it, vi } from 'vitest';
import RuleBuilder from './RuleBuilder.vue';

const backend = vi.hoisted(() => ({
  backendListRules: vi.fn(), backendUpsertRule: vi.fn(), backendDeleteRule: vi.fn(),
  backendRunRulesForThread: vi.fn(), backendRunRulesForThreads: vi.fn(),
  backendGetServerRulesOverview: vi.fn(), backendListMailboxes: vi.fn(), backendResumeRuleForwarding: vi.fn(),
}));
vi.mock('@/services/backend', () => backend);
const confirm = vi.hoisted(() => vi.fn());
vi.mock('@/composables/useConfirm', () => ({ useConfirm: () => ({ ask: confirm }) }));
vi.mock('@/stores/threads', () => ({ useThreadsStore: () => ({ visibleThreads: [], activeThreadId: '' }) }));
vi.mock('@/stores/accounts', () => ({
  useAccountsStore: () => ({
    accounts: [
      { id: 'work', email: 'me@work.example.test', kind: 'email' },
      { id: 'home', email: 'me@home.example.test', kind: 'email' },
      { id: 'feed', email: 'feed', kind: 'rss' },
    ],
  }),
}));
vi.mock('@/stores/aliases', () => ({
  useAliasesStore: () => ({ items: [{ id: 'a', account_id: 'work', email: 'billing@work.example.test' }], load: vi.fn() }),
}));
const mailbox = (id: string, account: string, name: string, type = 'custom') =>
  ({ id, account_id: account, account_email: '', name, mailbox_type: type, thread_count: 0, unread_count: 0 });
beforeEach(() => {
  vi.resetAllMocks();
  backend.backendListRules.mockResolvedValue([]);
  backend.backendListMailboxes.mockResolvedValue([
    mailbox('work-inbox', 'work', 'INBOX', 'inbox'),
    mailbox('work-billing', 'work', 'Billing'),
    mailbox('work-sent', 'work', 'Sent', 'sent'),
    mailbox('home-family', 'home', 'Family'),
  ]);
});

async function newRule(name: string) {
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  await wrapper.findAll('button').find((b) => b.text() === 'New rule')!.trigger('click');
  await wrapper.get('form > label input').setValue(name);
  await wrapper.findAll('fieldset')[0]!.get('.flow-row input').setValue('billing@');
  return wrapper;
}

const actionRow = (wrapper: Awaited<ReturnType<typeof newRule>>) => wrapper.findAll('fieldset')[1]!.get('.flow-row');

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

it('forwards to one valid address, with or without a server copy', async () => {
  const wrapper = await newRule('Books');
  await actionRow(wrapper).get('select').setValue('forward');
  const address = wrapper.get('[data-testid="rule-forward-address"]');
  const save = () => wrapper.findAll('button').find((b) => b.text() === 'Save')!;

  await address.setValue('Books <books@example.test>');
  expect(wrapper.get('.field-error').text()).toBe('Enter one email address, without a name.');
  expect(address.attributes('aria-invalid')).toBe('true');
  expect(save().attributes('disabled')).toBeDefined();

  await address.setValue('Me+lists@work.example.test');
  expect(wrapper.get('.field-error').text()).toContain('one of your own addresses');
  await address.setValue('billing+x@work.example.test');
  expect(wrapper.get('.field-error').text()).toContain('one of your own addresses'); // an alias

  await address.setValue('books@example.test');
  expect(wrapper.find('.field-error').exists()).toBe(false);
  expect(save().attributes('disabled')).toBeUndefined();
  // Turning the copy off is confirmed; declined, it stays on.
  confirm.mockResolvedValueOnce(false);
  await actionRow(wrapper).get('input[type="checkbox"]').setValue(false);
  await flushPromises();
  expect(confirm).toHaveBeenCalledWith(expect.objectContaining({ title: 'Forward without keeping a copy?', tone: 'danger' }));
  expect((actionRow(wrapper).get('input[type="checkbox"]').element as HTMLInputElement).checked).toBe(true);
  confirm.mockResolvedValueOnce(true);
  await actionRow(wrapper).get('input[type="checkbox"]').setValue(false);
  await flushPromises();
  expect(wrapper.text()).toContain('if the forward fails (the address bounces), the message is gone from your mailbox');
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(backend.backendUpsertRule).toHaveBeenCalledWith(expect.objectContaining({
    actions: ['forward_no_copy:books@example.test'],
    account_id: null,
  }));
});

it('moves into a folder of the chosen account, never its Inbox or Sent', async () => {
  const wrapper = await newRule('Billing');
  await actionRow(wrapper).get('select').setValue('move_to_folder');
  expect(actionRow(wrapper).text()).toContain('Choose an account above');
  expect(wrapper.findAll('button').find((b) => b.text() === 'Save')!.attributes('disabled')).toBeDefined();

  await wrapper.get('[data-testid="rule-account"]').setValue('work');
  const folders = wrapper.get('[data-testid="rule-folder"]').findAll('option').map((option) => option.text());
  expect(folders).toEqual(['Choose a folder', 'Billing']);
  await wrapper.get('[data-testid="rule-folder"]').setValue('work-billing');
  expect(wrapper.text()).toContain('the first matching rule decides the folder');

  await wrapper.findAll('button').find((b) => b.text() === 'Add action')!.trigger('click');
  await wrapper.findAll('fieldset')[1]!.findAll('.flow-row')[1]!.get('select').setValue('star');
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(backend.backendUpsertRule).toHaveBeenCalledWith(expect.objectContaining({
    actions: ['move_to_folder:work-billing', 'star'],
    account_id: 'work',
  }));
});

it('shows a translated backend refusal instead of its key', async () => {
  backend.backendUpsertRule.mockRejectedValue('backend.errors.rules.actionsInvalid');
  const wrapper = await newRule('Refused');
  await actionRow(wrapper).get('select').setValue('star');
  await wrapper.get('form').trigger('submit');
  await flushPromises();
  expect(wrapper.get('[role="alert"]').text()).toContain("This rule's folder or forward address is not allowed.");
});

it('marks each saved action as running on the server, pending or staying local', async () => {
  backend.backendListRules.mockResolvedValue([{
    id: 'r', name: 'Mixed', priority: 0, is_active: true, account_id: 'work',
    trigger_config: { from_contains: 'billing@' },
    actions: ['star', 'move_to_folder:work-billing', 'forward:books@example.test'],
    forward_paused_at: null, server_forward_skips: 0,
  }]);
  backend.backendGetServerRulesOverview.mockResolvedValue({
    accounts: [{ account_id: 'work', display_name: 'Work', email: 'me@work.example.test', enabled: true, server_active: true }],
    rules: [{
      rule_id: 'r', server_account_ids: [], pending: false,
      local_reason: null,
      actions: [
        { state: 'server', reason: null },
        { state: 'pending', reason: null },
        { state: 'local', reason: { kind: 'extension', detail: 'copy' } },
      ],
    }],
  });
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  expect(wrapper.text()).toContain('Move to Billing');
  expect(wrapper.text()).toContain('Forward to books@example.test');
  await wrapper.findAll('button').find((b) => b.text() === 'Edit')!.trigger('click');
  const badges = wrapper.findAll('[data-testid="action-placement"]');
  expect(badges.map((badge) => badge.text())).toEqual(['Server', 'Not on the server yet', 'Local only']);
  expect(badges[2]!.attributes('title')).toBe('The server lacks the Sieve extension “copy”.');
  // An action changed in the editor is no longer what the server answered for.
  await wrapper.findAll('fieldset')[1]!.findAll('.flow-row')[0]!.get('select').setValue('mark_read');
  expect(wrapper.findAll('[data-testid="action-placement"]')[0]!.text()).toBe('');
});

it('shows a rule whose folder or account is gone as broken, by name not id', async () => {
  backend.backendListRules.mockResolvedValue([
    { id: 'gone-folder', name: 'Old folder', priority: 0, is_active: true, account_id: 'work',
      trigger_config: { from_contains: 'x' }, actions: ['move_to_folder:work-deleted'], forward_paused_at: null, server_forward_skips: 0 },
    { id: 'gone-account', name: 'Old account', priority: 0, is_active: true, account_id: 'removed',
      trigger_config: { from_contains: 'x' }, actions: ['star'], forward_paused_at: null, server_forward_skips: 0 },
  ]);
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  const broken = wrapper.findAll('[data-testid="rule-broken"]').map((badge) => badge.text());
  expect(broken).toEqual(['Folder missing: the move does nothing', 'Account removed: the rule no longer runs']);
  expect(wrapper.text()).toContain('Move to a folder that no longer exists');
  expect(wrapper.text()).not.toContain('work-deleted');
});

it('says when forwarding is paused or left to the server, and resumes it on request', async () => {
  backend.backendListRules.mockResolvedValue([{
    id: 'r', name: 'Books', priority: 0, is_active: true, account_id: null,
    trigger_config: { from_contains: 'billing@' }, actions: ['forward:books@example.test'],
    forward_paused_at: '2026-11-07T10:00:00Z', server_forward_skips: 3,
  }]);
  const wrapper = mount(RuleBuilder);
  await flushPromises();
  expect(wrapper.get('[data-testid="rule-forward-paused"]').text()).toContain('more than 20 messages within an hour');
  expect(wrapper.get('[data-testid="rule-server-skips"]').text()).toBe(
    '3 forwards were left to the server, which ran this rule when the messages arrived.');
  expect(wrapper.get('[data-testid="rule-forward-paused"]').text()).toContain('Saving the rule leaves it paused.');
  await wrapper.findAll('button').find((b) => b.text() === 'Resume forwarding')!.trigger('click');
  await flushPromises();
  expect(backend.backendResumeRuleForwarding).toHaveBeenCalledWith('r');
  expect(backend.backendUpsertRule).not.toHaveBeenCalled();
});

it('says why Save is off with a second move or forward', async () => {
  const wrapper = await newRule('Twice');
  await actionRow(wrapper).get('select').setValue('forward');
  await wrapper.get('[data-testid="rule-forward-address"]').setValue('books@example.test');
  expect(wrapper.find('[data-testid="rule-one-target"]').exists()).toBe(false);
  await wrapper.findAll('button').find((b) => b.text() === 'Add action')!.trigger('click');
  await wrapper.findAll('fieldset')[1]!.findAll('.flow-row')[1]!.get('select').setValue('forward');
  expect(wrapper.get('[data-testid="rule-one-target"]').text()).toBe(
    'A rule can move to one folder and forward to one address. Remove the extra action to save.');
  expect(wrapper.findAll('button').find((b) => b.text() === 'Save')!.attributes('disabled')).toBeDefined();
});
