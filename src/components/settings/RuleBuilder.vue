<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendDeleteRule,
  backendListRules,
  backendRunRulesForThread,
  backendRunRulesForThreads,
  backendUpsertRule,
  type BackendRule,
} from '@/services/backend';
import { useThreadsStore } from '@/stores/threads';
import AppButton from '@/components/ui/AppButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

type ConditionField = 'from_contains' | 'to_contains' | 'subject_contains' | 'body_contains';
type ActionKind = 'set_state' | 'set_priority' | 'set_category' | 'add_tag' | 'mark_read';

interface ConditionDraft { field: ConditionField; value: string }
interface ActionDraft { kind: ActionKind; value: string }

const { t } = useI18n();

const conditionFields: ConditionField[] = [
  'from_contains',
  'to_contains',
  'subject_contains',
  'body_contains',
];
const actionKinds: ActionKind[] = [
  'set_state',
  'set_priority',
  'set_category',
  'add_tag',
  'mark_read',
];

const conditionLabels = computed<Record<ConditionField, string>>(() => ({
  from_contains: t('settings.rules.conditions.fromContains'),
  to_contains: t('settings.rules.conditions.toContains'),
  subject_contains: t('settings.rules.conditions.subjectContains'),
  body_contains: t('settings.rules.conditions.bodyContains'),
}));

const actionLabels = computed<Record<ActionKind, string>>(() => ({
  set_state: t('settings.rules.actions.setState'),
  set_priority: t('settings.rules.actions.setPriority'),
  set_category: t('settings.rules.actions.setCategory'),
  add_tag: t('settings.rules.actions.addTag'),
  mark_read: t('settings.rules.actions.markRead'),
}));

const threads = useThreadsStore();
const rules = ref<BackendRule[]>([]);
const loading = ref(false);
const saving = ref(false);
const error = ref('');
const notice = ref('');
const editingId = ref<string | null>(null);
const editorOpen = ref(false);
const name = ref('');
const description = ref('');
const matchMode = ref<'all' | 'any'>('all');
const priority = ref(0);
const active = ref(true);
const conditions = ref<ConditionDraft[]>([{ field: 'from_contains', value: '' }]);
const actions = ref<ActionDraft[]>([{ kind: 'set_category', value: 'updates' }]);

const canSave = computed(() =>
  name.value.trim().length > 0
  && conditions.value.some((condition) => condition.value.trim().length > 0)
  && actions.value.every((action) => action.kind === 'mark_read' || action.value.trim().length > 0),
);

async function load() {
  loading.value = true;
  error.value = '';
  try {
    rules.value = await backendListRules();
  } catch (err) {
    error.value = t('settings.rules.loadFailed', { error: String(err) });
  } finally {
    loading.value = false;
  }
}

function resetEditor() {
  editingId.value = null;
  name.value = '';
  description.value = '';
  matchMode.value = 'all';
  priority.value = 0;
  active.value = true;
  conditions.value = [{ field: 'from_contains', value: '' }];
  actions.value = [{ kind: 'set_category', value: 'updates' }];
}

function startCreate() {
  resetEditor();
  editorOpen.value = true;
  notice.value = '';
}

function startFromActiveThread() {
  startCreate();
  const activeThread = threads.find(threads.activeThreadId);
  const sender = activeThread?.participants[0];
  if (!sender) return;
  name.value = t('settings.rules.nameFromSender', { sender: sender.name || sender.email });
  conditions.value = [{ field: 'from_contains', value: sender.email }];
}

function editRule(rule: BackendRule) {
  editingId.value = rule.id;
  name.value = rule.name;
  description.value = rule.description ?? '';
  priority.value = rule.priority;
  active.value = rule.is_active;
  matchMode.value = rule.trigger_config.match === 'any' ? 'any' : 'all';
  conditions.value = conditionFields
    .filter((field) => typeof rule.trigger_config[field] === 'string')
    .map((field) => ({ field, value: String(rule.trigger_config[field]) }));
  if (conditions.value.length === 0) conditions.value = [{ field: 'from_contains', value: '' }];
  actions.value = rule.actions.map(parseAction);
  if (actions.value.length === 0) actions.value = [{ kind: 'set_category', value: 'updates' }];
  editorOpen.value = true;
  notice.value = '';
}

function parseAction(action: string): ActionDraft {
  if (action === 'mark_read') return { kind: 'mark_read', value: '' };
  const [kind, ...rest] = action.split(':');
  if (kind && actionKinds.includes(kind as ActionKind)) {
    return { kind: kind as ActionKind, value: rest.join(':') };
  }
  return { kind: 'add_tag', value: action };
}

function serializeAction(action: ActionDraft): string {
  return action.kind === 'mark_read' ? 'mark_read' : `${action.kind}:${action.value.trim()}`;
}

function actionDefault(kind: ActionKind): string {
  if (kind === 'set_state') return 'done';
  if (kind === 'set_priority') return '2';
  if (kind === 'set_category') return 'updates';
  return '';
}

function changeActionKind(action: ActionDraft, event: Event) {
  action.kind = (event.target as HTMLSelectElement).value as ActionKind;
  action.value = actionDefault(action.kind);
}

function addCondition() {
  const unused = conditionFields
    .find((field) => !conditions.value.some((condition) => condition.field === field));
  conditions.value.push({ field: unused ?? 'subject_contains', value: '' });
}

function addAction() {
  actions.value.push({ kind: 'mark_read', value: '' });
}

async function save() {
  if (!canSave.value || saving.value) return;
  saving.value = true;
  error.value = '';
  const triggerConfig: Record<string, string> = { match: matchMode.value };
  for (const condition of conditions.value) {
    const value = condition.value.trim();
    if (value) triggerConfig[condition.field] = value;
  }
  try {
    await backendUpsertRule({
      id: editingId.value,
      name: name.value.trim(),
      description: description.value.trim() || null,
      trigger_type: 'on_receive',
      trigger_config: triggerConfig,
      actions: actions.value.map(serializeAction),
      is_active: active.value,
      priority: priority.value,
    });
    editorOpen.value = false;
    notice.value = t('settings.rules.saved');
    await load();
  } catch (err) {
    error.value = t('settings.rules.saveFailed', { error: String(err) });
  } finally {
    saving.value = false;
  }
}

async function toggleRule(rule: BackendRule) {
  await backendUpsertRule({ ...rule, id: rule.id, is_active: !rule.is_active });
  await load();
}

async function removeRule(rule: BackendRule) {
  if (!window.confirm(t('settings.rules.deleteConfirm', { name: rule.name }))) return;
  await backendDeleteRule(rule.id);
  await load();
}

async function runNow() {
  if (!threads.activeThreadId) return;
  try {
    const applied = await backendRunRulesForThread(threads.activeThreadId);
    notice.value = applied.length > 0
      ? t('settings.rules.appliedCount', applied.length)
      : t('settings.rules.noRuleMatched');
    await threads.load();
  } catch (err) {
    error.value = t('settings.rules.runFailed', { error: String(err) });
  }
}

async function runVisible() {
  const threadIds = threads.visibleThreads.map((thread) => thread.id);
  if (threadIds.length === 0) return;
  if (!window.confirm(t('settings.rules.applyVisibleConfirm', threadIds.length))) return;
  try {
    const applied = await backendRunRulesForThreads(threadIds);
    notice.value = t('settings.rules.appliedCount', applied.length);
    await threads.load();
  } catch (err) {
    error.value = t('settings.rules.runFailed', { error: String(err) });
  }
}

function ruleSummary(rule: BackendRule): string {
  const summary = conditionFields
    .filter((field) => rule.trigger_config[field])
    .map((field) => t('settings.rules.summaryCondition', {
      label: conditionLabels.value[field],
      value: rule.trigger_config[field],
    }));
  return summary.join(t(rule.trigger_config.match === 'any'
    ? 'settings.rules.summaryOr'
    : 'settings.rules.summaryAnd'));
}

onMounted(load);
</script>

<template>
  <section class="rules-section">
    <header class="section-header">
      <div>
        <h3>{{ $t('settings.rules.title') }}</h3>
        <p>{{ $t('settings.rules.description') }}</p>
      </div>
      <AppButton variant="primary" @click="startCreate">{{ $t('settings.rules.newRule') }}</AppButton>
    </header>

    <p v-if="error" class="message error">{{ error }}</p>
    <p v-if="notice" class="message notice">{{ notice }}</p>
    <p v-if="loading" class="empty">{{ $t('settings.rules.loading') }}</p>
    <p v-else-if="rules.length === 0 && !editorOpen" class="empty">{{ $t('settings.rules.empty') }}</p>

    <div class="rule-list">
      <article v-for="rule in rules" :key="rule.id" class="rule-card">
        <div class="rule-copy">
          <div class="rule-title">
            <span :class="['status', { active: rule.is_active }]">{{ rule.is_active ? $t('common.enabled') : $t('settings.rules.paused') }}</span>
            <strong>{{ rule.name }}</strong>
          </div>
          <div class="rule-flow-preview">
            <span class="flow-kicker condition">{{ $t('settings.rules.when') }}</span>
            <span class="flow-copy">{{ ruleSummary(rule) }}</span>
            <svg class="flow-arrow" viewBox="0 0 20 20" aria-hidden="true"><path d="M3 10h13m-4-4 4 4-4 4" /></svg>
            <span class="flow-kicker action">{{ $t('settings.rules.then') }}</span>
            <span class="flow-copy">{{ rule.actions.map(parseAction).map((action) => actionLabels[action.kind]).join(' · ') }}</span>
          </div>
        </div>
        <div class="rule-actions">
          <AppButton size="sm" @click="void toggleRule(rule)">
            {{ rule.is_active ? $t('settings.rules.pause') : $t('settings.rules.activate') }}
          </AppButton>
          <AppButton size="sm" @click="editRule(rule)">{{ $t('common.edit') }}</AppButton>
          <AppButton variant="danger" size="sm" @click="void removeRule(rule)">{{ $t('common.delete') }}</AppButton>
        </div>
      </article>
    </div>

    <form v-if="editorOpen" class="editor" @submit.prevent="save">
      <div class="editor-heading">
        <h4>{{ editingId ? $t('settings.rules.editTitle') : $t('settings.rules.createTitle') }}</h4>
        <ToggleSwitch v-model="active">{{ $t('common.enabled') }}</ToggleSwitch>
      </div>
      <label>{{ $t('common.name') }} <input v-model="name" maxlength="160" required /></label>
      <label>{{ $t('common.description') }} <input v-model="description" maxlength="500" /></label>
      <label>{{ $t('settings.rules.priority') }} <input v-model.number="priority" type="number" min="-1000" max="1000" /></label>

      <fieldset>
        <legend>{{ $t('settings.rules.conditionsLegend') }}</legend>
        <label class="inline">{{ $t('settings.rules.matchMode') }}
          <select v-model="matchMode"><option value="all">{{ $t('settings.rules.matchAll') }}</option><option value="any">{{ $t('settings.rules.matchAny') }}</option></select>
        </label>
        <div v-for="(condition, index) in conditions" :key="index" class="flow-row">
          <select v-model="condition.field">
            <option v-for="(label, field) in conditionLabels" :key="field" :value="field">{{ label }}</option>
          </select>
          <input v-model="condition.value" :placeholder="$t('settings.rules.valuePlaceholder')" />
          <IconButton
            :title="$t('settings.rules.removeCondition')"
            :disabled="conditions.length === 1"
            @click="conditions.splice(index, 1)"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
              <path d="M3.5 8h9" />
            </svg>
          </IconButton>
        </div>
        <AppButton size="sm" @click="addCondition">{{ $t('settings.rules.addCondition') }}</AppButton>
      </fieldset>

      <fieldset>
        <legend>{{ $t('settings.rules.actionsLegend') }}</legend>
        <div v-for="(action, index) in actions" :key="index" class="flow-row">
          <select :value="action.kind" @change="changeActionKind(action, $event)">
            <option v-for="(label, kind) in actionLabels" :key="kind" :value="kind">{{ label }}</option>
          </select>
          <select v-if="action.kind === 'set_state'" v-model="action.value"><option value="inbox">{{ $t('settings.rules.states.inbox') }}</option><option value="todo">{{ $t('settings.rules.states.todo') }}</option><option value="waiting">{{ $t('settings.rules.states.waiting') }}</option><option value="aside">{{ $t('settings.rules.states.aside') }}</option><option value="done">{{ $t('settings.rules.states.done') }}</option></select>
          <select v-else-if="action.kind === 'set_priority'" v-model="action.value"><option value="0">{{ $t('common.none') }}</option><option value="1">{{ $t('settings.rules.priorities.low') }}</option><option value="2">{{ $t('settings.rules.priorities.medium') }}</option><option value="3">{{ $t('settings.rules.priorities.high') }}</option></select>
          <select v-else-if="action.kind === 'set_category'" v-model="action.value"><option value="personal">{{ $t('settings.rules.categories.personal') }}</option><option value="updates">{{ $t('settings.rules.categories.updates') }}</option><option value="promotions">{{ $t('settings.rules.categories.promotions') }}</option><option value="social">{{ $t('settings.rules.categories.social') }}</option><option value="none">{{ $t('common.none') }}</option></select>
          <input v-else-if="action.kind === 'add_tag'" v-model="action.value" maxlength="80" :placeholder="$t('settings.rules.tagPlaceholder')" />
          <span v-else class="action-hint">{{ $t('settings.rules.noExtraInput') }}</span>
          <IconButton
            :title="$t('settings.rules.removeAction')"
            :disabled="actions.length === 1"
            @click="actions.splice(index, 1)"
          >
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
              <path d="M3.5 8h9" />
            </svg>
          </IconButton>
        </div>
        <AppButton size="sm" @click="addAction">{{ $t('settings.rules.addAction') }}</AppButton>
      </fieldset>

      <div class="editor-actions">
        <AppButton @click="editorOpen = false">{{ $t('common.cancel') }}</AppButton>
        <AppButton variant="primary" type="submit" :disabled="!canSave || saving">
          {{ saving ? $t('common.saving') : $t('common.save') }}
        </AppButton>
      </div>
    </form>

    <div class="run-actions">
      <AppButton v-if="threads.activeThreadId" size="sm" @click="startFromActiveThread">
        {{ $t('settings.rules.createFromOpenMail') }}
      </AppButton>
      <AppButton v-if="threads.activeThreadId" size="sm" @click="void runNow">
        {{ $t('settings.rules.applyToOpenMail') }}
      </AppButton>
      <AppButton size="sm" @click="void runVisible">{{ $t('settings.rules.applyToVisibleMails') }}</AppButton>
    </div>
  </section>
</template>

<style scoped>
.rules-section { display: grid; gap: 16px; }
.section-header, .editor-heading, .rule-card, .rule-title, .rule-actions, .editor-actions, .inline { display: flex; align-items: center; }
.section-header, .editor-heading, .rule-card { justify-content: space-between; gap: 16px; }
h3, h4, p { margin: 0; }
.section-header p, .empty { color: var(--text-tertiary); }
.section-header p { margin-top: 4px; font-size: var(--font-size-sm); }
input, select { border: 1px solid var(--border-color-muted); border-radius: var(--radius-sm); background: var(--bg-raised); color: var(--text-primary); padding: 7px 10px; }
.message.error { color: var(--accent-red); }
.message { padding: 10px 12px; border-radius: var(--radius-sm); background: var(--bg-raised); }
.message.notice { color: var(--accent-green); }
.rule-list { display: grid; gap: 8px; }
.rule-card, .editor { padding: 14px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-md); background: var(--bg-surface); }
.rule-copy { min-width: 0; display: grid; gap: 5px; }
.rule-title, .rule-actions, .editor-actions { gap: 8px; }
.status { padding: 2px 7px; border-radius: var(--radius-full); background: var(--bg-overlay); color: var(--text-tertiary); font-size: var(--font-size-2xs); text-transform: uppercase; }
.status.active { color: var(--accent-green); background: color-mix(in srgb, var(--accent-green) 14%, transparent); }
.rule-flow-preview { display: grid; grid-template-columns: auto minmax(0, 1fr) auto auto minmax(0, 1fr); align-items: center; gap: 8px; margin-top: 3px; }
.flow-kicker { padding: 3px 7px; border-radius: var(--radius-full); font-size: var(--font-size-2xs); font-weight: var(--weight-bold); letter-spacing: .035em; text-transform: uppercase; }
.flow-kicker.condition { color: var(--accent-amber); background: color-mix(in srgb, var(--accent-amber) 12%, transparent); }
.flow-kicker.action { color: var(--accent-green); background: color-mix(in srgb, var(--accent-green) 12%, transparent); }
.flow-copy { min-width: 0; overflow: hidden; color: var(--text-secondary); font-size: var(--font-size-xs); line-height: 1.45; text-overflow: ellipsis; white-space: nowrap; }
.flow-arrow { width: 17px; color: var(--text-tertiary); fill: none; stroke: currentColor; stroke-linecap: round; stroke-linejoin: round; stroke-width: 1.35; }
.editor { display: grid; gap: 12px; }
.editor > label { display: grid; gap: 5px; font-size: var(--font-size-sm); }
fieldset { display: grid; gap: 9px; margin: 0; padding: 12px; border: 1px solid var(--border-color-muted); border-radius: var(--radius-sm); }
legend { padding: 0 6px; font-weight: var(--weight-semibold); }
.inline { justify-content: flex-start; gap: 8px; }
.flow-row { display: grid; grid-template-columns: minmax(150px, .8fr) minmax(180px, 1fr) auto; gap: 8px; }
.action-hint { padding: 8px 10px; color: var(--text-tertiary); }
.editor-actions { justify-content: flex-end; }
.run-actions { display: flex; flex-wrap: wrap; gap: 8px; }
@media (max-width: 700px) { .section-header, .rule-card { align-items: stretch; flex-direction: column; } .flow-row { grid-template-columns: 1fr; } .rule-actions { flex-wrap: wrap; } .rule-flow-preview { grid-template-columns: auto minmax(0, 1fr); } .flow-arrow { display: none; } }
</style>
