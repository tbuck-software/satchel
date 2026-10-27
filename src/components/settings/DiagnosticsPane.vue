<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import {
  backendAppBuildInfo,
  backendCommandsAvailable,
  backendDiagnosticsOverview,
  backendDiagnosticsReport,
  backendOpenLogsFolder,
  translateBackendError,
  type BackendBuildInfo,
  type BackendDiagnosticsOverview,
} from '@/services/backend';
import AppButton from '@/components/ui/AppButton.vue';
import EmptyState from '@/components/ui/EmptyState.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { useToast } from '@/composables/useToast';

const { locale, t } = useI18n();
const toast = useToast();
const overview = ref<BackendDiagnosticsOverview | null>(null);
const build = ref<BackendBuildInfo | null>(null);
const loading = ref(false);
const copying = ref(false);
const status = ref('');

// Phones have no file manager to open a folder in.
const canOpenLogs = computed(
  () => build.value !== null && ['macos', 'windows', 'linux'].includes(build.value.os),
);

// Fixed at load: the pane is a snapshot, and a ticking "vor N Min." would only
// suggest the numbers below it are live too.
const now = ref(Date.now());

const absoluteFormatter = computed(
  () => new Intl.DateTimeFormat(locale.value, {
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  }),
);

function relativeTime(value: string | null | undefined): string {
  if (!value) return '—';
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return '—';

  const elapsed = now.value - parsed;
  if (elapsed < 0) return t('settings.diagnostics.time.in', { span: describeSpan(-elapsed) });
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 1) return t('settings.diagnostics.time.justNow');
  if (minutes < 60) return t('settings.diagnostics.time.minutesAgo', { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t('settings.diagnostics.time.hoursAgo', { count: hours });
  const days = Math.floor(hours / 24);
  if (days < 7) return t('settings.diagnostics.time.daysAgo', { count: days }, days);
  return absoluteFormatter.value.format(new Date(parsed));
}

function describeSpan(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return t('settings.diagnostics.time.lessThanAMinute');
  if (minutes < 60) return t('settings.diagnostics.time.minutes', { count: minutes });
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return t('settings.diagnostics.time.hours', { count: hours });
  const days = Math.floor(hours / 24);
  return t('settings.diagnostics.time.days', { count: days }, days);
}

function absoluteTime(value: string | null | undefined): string {
  if (!value) return '';
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? '' : absoluteFormatter.value.format(new Date(parsed));
}

function syncStatusLabel(value: string): string {
  if (value === 'idle' || value === 'syncing' || value === 'error') {
    return t(`settings.diagnostics.syncStatus.${value}`);
  }
  return value;
}

function queueStatusLabel(value: string): string {
  if (
    value === 'pending'
    || value === 'scheduled'
    || value === 'sending'
    || value === 'failed'
  ) {
    return t(`settings.diagnostics.queueStatus.${value}`);
  }
  return value;
}

function outboxStateLabel(value: string): string {
  switch (value) {
    case 'pending': return t('settings.diagnostics.outbox.states.pending');
    case 'dispatching': return t('settings.diagnostics.outbox.states.dispatching');
    case 'delivered': return t('settings.diagnostics.outbox.states.delivered');
    case 'dismissed': return t('settings.diagnostics.outbox.states.dismissed');
    default: return value;
  }
}

// Mirrors `commands::notifications::reason` in the backend. An unknown code is
// shown verbatim rather than hidden. It is what the user would report.
function reasonLabel(value: string | null): string {
  if (!value) return '—';
  switch (value) {
    case 'setting_disabled': return t('settings.diagnostics.outbox.reasons.setting_disabled');
    case 'filter_message_gone': return t('settings.diagnostics.outbox.reasons.filter_message_gone');
    case 'filter_account_resting': return t('settings.diagnostics.outbox.reasons.filter_account_resting');
    case 'filter_account_inactive': return t('settings.diagnostics.outbox.reasons.filter_account_inactive');
    case 'filter_not_inbox': return t('settings.diagnostics.outbox.reasons.filter_not_inbox');
    case 'filter_own_sent': return t('settings.diagnostics.outbox.reasons.filter_own_sent');
    case 'filter_already_read': return t('settings.diagnostics.outbox.reasons.filter_already_read');
    case 'filter_muted_thread': return t('settings.diagnostics.outbox.reasons.filter_muted_thread');
    case 'filter_muted_sender': return t('settings.diagnostics.outbox.reasons.filter_muted_sender');
    case 'filter_no_date': return t('settings.diagnostics.outbox.reasons.filter_no_date');
    case 'filter_before_session': return t('settings.diagnostics.outbox.reasons.filter_before_session');
    case 'filter_too_old': return t('settings.diagnostics.outbox.reasons.filter_too_old');
    case 'dispatch_timeout': return t('settings.diagnostics.outbox.reasons.dispatch_timeout');
    case 'dispatch_error': return t('settings.diagnostics.outbox.reasons.dispatch_error');
    case 'permission_denied': return t('settings.diagnostics.outbox.reasons.permission_denied');
    case 'not_bundled_fallback': return t('settings.diagnostics.outbox.reasons.not_bundled_fallback');
    case 'filter_coalesced': return t('settings.diagnostics.outbox.reasons.filter_coalesced');
    default: return value;
  }
}

function senderLabel(entry: { from_name: string | null; from_address: string | null }): string {
  const name = entry.from_name?.trim();
  if (name) return name;
  return entry.from_address?.trim() || '—';
}

const reasonTotal24h = computed(
  () => overview.value?.notification_outbox.reasons_24h.reduce((sum, r) => sum + r.entries, 0) ?? 0,
);

const accountsWithProblems = computed(
  () => overview.value?.accounts.filter(
    (account) =>
      account.last_sync_error
      || account.last_oauth_error
      || account.sync_status === 'error',
  ) ?? [],
);

const activeRateLimits = computed(
  () => overview.value?.rate_limits.filter((limit) => limit.blocked_until !== null) ?? [],
);

async function load() {
  if (!backendCommandsAvailable()) return;
  loading.value = true;
  status.value = '';
  try {
    overview.value = await backendDiagnosticsOverview();
    now.value = Date.now();
  } catch (err) {
    status.value = err instanceof Error ? err.message : String(err);
  } finally {
    loading.value = false;
  }
}

async function loadBuild() {
  if (!backendCommandsAvailable()) return;
  try {
    build.value = await backendAppBuildInfo();
  } catch {
    // The pane is still useful without the build line.
  }
}

async function copyReport() {
  copying.value = true;
  try {
    await navigator.clipboard.writeText(await backendDiagnosticsReport());
    toast.success(t('settings.diagnostics.report.copied'));
  } catch (err) {
    toast.error(translateBackendError(err) ?? t('settings.diagnostics.report.copyFailed'));
  } finally {
    copying.value = false;
  }
}

async function openLogs() {
  try {
    await backendOpenLogsFolder();
  } catch (err) {
    toast.error(translateBackendError(err) ?? t('backend.errors.logsFolderUnavailable'));
  }
}

onMounted(() => {
  void load();
  void loadBuild();
});
</script>

<template>
  <section class="diagnostics">
    <header>
      <div>
        <h3>{{ $t('settings.sections.diagnostics.label') }}</h3>
        <p class="hint">
          {{ $t('settings.diagnostics.intro') }}
          <template v-if="overview">
            {{
              $t('settings.diagnostics.generatedAt', {
                time: absoluteTime(overview.generated_at),
              })
            }}
          </template>
        </p>
      </div>
      <AppButton variant="soft" :disabled="loading" @click="load">
        {{ loading ? $t('settings.diagnostics.loading') : $t('common.refresh') }}
      </AppButton>
    </header>

    <div class="support">
      <dl v-if="build" class="meta" data-testid="build-info">
        <div>
          <dt>{{ $t('settings.diagnostics.build.version') }}</dt>
          <dd>{{ build.version }}</dd>
        </div>
        <div>
          <dt>{{ $t('settings.diagnostics.build.commit') }}</dt>
          <dd class="mono">{{ build.commit }}</dd>
        </div>
        <div>
          <dt>{{ $t('settings.diagnostics.build.platform') }}</dt>
          <dd>{{ build.os }}/{{ build.arch }}</dd>
        </div>
      </dl>
      <div class="actions">
        <AppButton
          variant="soft"
          :disabled="copying"
          :title="$t('settings.diagnostics.report.copyHint')"
          @click="copyReport"
        >
          {{ $t('settings.diagnostics.report.copy') }}
        </AppButton>
        <AppButton v-if="canOpenLogs" variant="soft" @click="openLogs">
          {{ $t('settings.diagnostics.report.openLogs') }}
        </AppButton>
      </div>
    </div>

    <p v-if="status" class="status">{{ status }}</p>

    <template v-if="overview">
      <article>
        <h4>{{ $t('settings.diagnostics.syncErrors.title') }}</h4>
        <EmptyState
          v-if="accountsWithProblems.length === 0"
          :title="$t('settings.diagnostics.syncErrors.emptyTitle')"
          :description="$t('settings.diagnostics.syncErrors.emptyDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M3.5 8.4 6.5 11.4 12.5 5" /></AppIcon>
          </template>
        </EmptyState>
        <ul v-else class="rows">
          <li v-for="account in accountsWithProblems" :key="account.id" class="row">
            <div class="row__head">
              <strong>{{ account.email }}</strong>
              <span class="tag" :class="{ 'tag--error': account.sync_status === 'error' }">
                {{ syncStatusLabel(account.sync_status) }}
              </span>
            </div>
            <p v-if="account.last_sync_error" class="row__error">
              {{ translateBackendError(account.last_sync_error) ?? account.last_sync_error }}
            </p>
            <p v-if="account.last_oauth_error" class="row__error">
              {{ $t('settings.diagnostics.authError', { error: account.last_oauth_error }) }}
            </p>
            <dl class="meta">
              <div>
                <dt>{{ $t('settings.diagnostics.lastSync') }}</dt>
                <dd :title="absoluteTime(account.last_sync_at)">
                  {{ relativeTime(account.last_sync_at) }}
                </dd>
              </div>
              <div v-if="account.last_sync_error_at">
                <dt>{{ $t('common.error') }}</dt>
                <dd :title="absoluteTime(account.last_sync_error_at)">
                  {{ relativeTime(account.last_sync_error_at) }}
                </dd>
              </div>
              <div v-if="account.last_oauth_error_at">
                <dt>{{ $t('settings.diagnostics.authErrorLabel') }}</dt>
                <dd :title="absoluteTime(account.last_oauth_error_at)">
                  {{ relativeTime(account.last_oauth_error_at) }}
                </dd>
              </div>
            </dl>
          </li>
        </ul>
      </article>

      <article>
        <h4>{{ $t('settings.diagnostics.queue.title') }}</h4>
        <EmptyState
          v-if="overview.queue.by_status.length === 0"
          :title="$t('settings.diagnostics.queue.emptyTitle')"
          :description="$t('settings.diagnostics.queue.emptyDescription')"
        />
        <table v-else class="table">
          <thead>
            <tr>
              <th>{{ $t('common.status') }}</th>
              <th class="num">{{ $t('settings.diagnostics.queue.entries') }}</th>
              <th class="num">{{ $t('settings.diagnostics.queue.totalRetries') }}</th>
              <th class="num">{{ $t('settings.diagnostics.queue.maxRetries') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="bucket in overview.queue.by_status" :key="bucket.status">
              <td>{{ queueStatusLabel(bucket.status) }}</td>
              <td class="num">{{ bucket.entries }}</td>
              <td class="num">{{ bucket.total_retries }}</td>
              <td class="num">{{ bucket.max_retries }}</td>
            </tr>
          </tbody>
        </table>

        <h5>{{ $t('settings.diagnostics.queue.recentFailures') }}</h5>
        <EmptyState
          v-if="overview.queue.recent_failures.length === 0"
          :title="$t('settings.diagnostics.queue.noFailuresTitle')"
          :description="$t('settings.diagnostics.queue.noFailuresDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M3.5 8.4 6.5 11.4 12.5 5" /></AppIcon>
          </template>
        </EmptyState>
        <ul v-else class="rows">
          <li v-for="entry in overview.queue.recent_failures" :key="entry.id" class="row">
            <div class="row__head">
              <strong>{{ entry.action_type }}</strong>
              <span class="tag" :class="{ 'tag--error': entry.status === 'failed' }">
                {{ queueStatusLabel(entry.status) }}
              </span>
            </div>
            <p v-if="entry.error" class="row__error">
              {{ translateBackendError(entry.error) ?? entry.error }}
            </p>
            <dl class="meta">
              <div>
                <dt>{{ $t('settings.diagnostics.queue.attempts') }}</dt>
                <dd>{{ entry.attempts }}</dd>
              </div>
              <div>
                <dt>{{ $t('settings.diagnostics.queue.nextAttempt') }}</dt>
                <dd :title="absoluteTime(entry.next_due)">{{ relativeTime(entry.next_due) }}</dd>
              </div>
              <div>
                <dt>{{ $t('settings.diagnostics.queue.updated') }}</dt>
                <dd :title="absoluteTime(entry.updated_at)">{{ relativeTime(entry.updated_at) }}</dd>
              </div>
            </dl>
          </li>
        </ul>
      </article>

      <article data-testid="notification-outbox">
        <h4>{{ $t('settings.diagnostics.outbox.title') }}</h4>
        <p class="hint">{{ $t('settings.diagnostics.outbox.hint') }}</p>
        <EmptyState
          v-if="overview.notification_outbox.by_state.length === 0"
          :title="$t('settings.diagnostics.outbox.emptyTitle')"
          :description="$t('settings.diagnostics.outbox.emptyDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M5 7.2a3 3 0 0 1 6 0c0 2 .8 2.8 1.4 3.4H3.6C4.2 10 5 9.2 5 7.2z" /><path d="M6.8 12.4a1.4 1.4 0 0 0 2.4 0" /><path d="M2.4 2.4 13.6 13.6" /></AppIcon>
          </template>
        </EmptyState>
        <template v-else>
          <table class="table">
            <thead>
              <tr>
                <th>{{ $t('settings.diagnostics.outbox.state') }}</th>
                <th class="num">{{ $t('settings.diagnostics.queue.entries') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="bucket in overview.notification_outbox.by_state" :key="bucket.state">
                <td>{{ outboxStateLabel(bucket.state) }}</td>
                <td class="num">{{ bucket.entries }}</td>
              </tr>
            </tbody>
          </table>

          <h5>{{ $t('settings.diagnostics.outbox.reasons24h') }}</h5>
          <p v-if="overview.notification_outbox.reasons_24h.length === 0" class="hint">
            {{ $t('settings.diagnostics.outbox.noSuppressed') }}
          </p>
          <ul v-else class="histogram">
            <li
              v-for="bucket in overview.notification_outbox.reasons_24h"
              :key="bucket.reason"
              class="histogram__row"
            >
              <span class="histogram__label" :title="bucket.reason">
                {{ reasonLabel(bucket.reason) }}
              </span>
              <span
                class="histogram__bar"
                :style="{ width: `${Math.max(2, (bucket.entries / reasonTotal24h) * 100)}%` }"
              />
              <span class="histogram__count">{{ bucket.entries }}</span>
            </li>
          </ul>

          <h5>{{ $t('settings.diagnostics.outbox.recent') }}</h5>
          <ul class="rows">
            <li
              v-for="entry in overview.notification_outbox.recent"
              :key="entry.message_id"
              class="row"
            >
              <div class="row__head">
                <strong class="row__subject">
                  {{ entry.subject || $t('settings.diagnostics.outbox.noSubject') }}
                </strong>
                <span class="tag" :class="{ 'tag--error': entry.state === 'dismissed' }">
                  {{ outboxStateLabel(entry.state) }}
                </span>
              </div>
              <dl class="meta">
                <div>
                  <dt>{{ $t('settings.diagnostics.outbox.sender') }}</dt>
                  <dd :title="entry.from_address ?? ''">{{ senderLabel(entry) }}</dd>
                </div>
                <div>
                  <dt>{{ $t('settings.diagnostics.outbox.received') }}</dt>
                  <dd :title="absoluteTime(entry.date)">{{ relativeTime(entry.date) }}</dd>
                </div>
                <div>
                  <dt>{{ $t('settings.diagnostics.outbox.decided') }}</dt>
                  <dd :title="absoluteTime(entry.delivered_at ?? entry.created_at)">
                    {{ relativeTime(entry.delivered_at ?? entry.created_at) }}
                  </dd>
                </div>
                <div v-if="entry.reason">
                  <dt>{{ $t('settings.diagnostics.outbox.reason') }}</dt>
                  <dd :title="entry.detail ?? entry.reason">{{ reasonLabel(entry.reason) }}</dd>
                </div>
              </dl>
            </li>
          </ul>
        </template>
      </article>

      <article data-testid="notification-actions">
        <h4>{{ $t('settings.diagnostics.notifications.title') }}</h4>
        <p class="hint">{{ $t('settings.diagnostics.notifications.hint') }}</p>
        <EmptyState
          v-if="overview.notifications.by_status.length === 0"
          :title="$t('settings.diagnostics.notifications.emptyTitle')"
          :description="$t('settings.diagnostics.notifications.emptyDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M5 7.2a3 3 0 0 1 6 0c0 2 .8 2.8 1.4 3.4H3.6C4.2 10 5 9.2 5 7.2z" /><path d="M6.8 12.4a1.4 1.4 0 0 0 2.4 0" /></AppIcon>
          </template>
        </EmptyState>
        <template v-else>
          <table class="table">
            <thead>
              <tr>
                <th>{{ $t('common.status') }}</th>
                <th class="num">{{ $t('settings.diagnostics.queue.entries') }}</th>
                <th class="num">{{ $t('settings.diagnostics.queue.totalRetries') }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="bucket in overview.notifications.by_status" :key="bucket.status">
                <td>{{ queueStatusLabel(bucket.status) }}</td>
                <td class="num">{{ bucket.entries }}</td>
                <td class="num">{{ bucket.total_retries }}</td>
              </tr>
            </tbody>
          </table>

          <ul class="rows">
            <li v-for="entry in overview.notifications.recent" :key="entry.id" class="row">
              <div class="row__head">
                <strong>{{ entry.action_type }}</strong>
                <span class="tag" :class="{ 'tag--error': entry.status === 'failed' }">
                  {{ queueStatusLabel(entry.status) }}
                </span>
              </div>
              <p v-if="entry.error" class="row__error">{{ entry.error }}</p>
              <dl class="meta">
                <div>
                  <dt>{{ $t('settings.diagnostics.queue.attempts') }}</dt>
                  <dd>{{ entry.attempts }}</dd>
                </div>
                <div>
                  <dt>{{ $t('settings.diagnostics.queue.updated') }}</dt>
                  <dd :title="absoluteTime(entry.updated_at)">
                    {{ relativeTime(entry.updated_at) }}
                  </dd>
                </div>
              </dl>
            </li>
          </ul>
        </template>
      </article>

      <article data-testid="sync-leases">
        <h4>{{ $t('settings.diagnostics.leases.title') }}</h4>
        <p class="hint">{{ $t('settings.diagnostics.leases.hint') }}</p>
        <EmptyState
          v-if="overview.sync_leases.length === 0"
          :title="$t('settings.diagnostics.leases.emptyTitle')"
          :description="$t('settings.diagnostics.leases.emptyDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><rect x="3.2" y="7.2" width="9.6" height="6.4" rx="1.4" /><path d="M5.6 7.2V5.2a2.4 2.4 0 0 1 4.6-1" /></AppIcon>
          </template>
        </EmptyState>
        <ul v-else class="rows">
          <li v-for="lease in overview.sync_leases" :key="lease.account_id" class="row">
            <div class="row__head">
              <strong>{{ lease.email ?? lease.account_id }}</strong>
              <span class="tag" :class="{ 'tag--error': lease.expired }">
                {{ lease.expired
                  ? $t('settings.diagnostics.leases.expired')
                  : $t('settings.diagnostics.leases.active') }}
              </span>
            </div>
            <dl class="meta">
              <div>
                <dt>{{ $t('settings.diagnostics.leases.owner') }}</dt>
                <dd class="mono">{{ lease.owner_token }}</dd>
              </div>
              <div>
                <dt>{{ $t('settings.diagnostics.leases.expires') }}</dt>
                <dd :title="absoluteTime(lease.lease_expires_at)">
                  {{ relativeTime(lease.lease_expires_at) }}
                </dd>
              </div>
              <div>
                <dt>{{ $t('settings.diagnostics.leases.renewed') }}</dt>
                <dd :title="absoluteTime(lease.updated_at)">{{ relativeTime(lease.updated_at) }}</dd>
              </div>
            </dl>
          </li>
        </ul>
      </article>

      <article>
        <h4>{{ $t('settings.diagnostics.rateLimits.title') }}</h4>
        <EmptyState
          v-if="activeRateLimits.length === 0"
          :title="$t('settings.diagnostics.rateLimits.emptyTitle')"
          :description="$t('settings.diagnostics.rateLimits.emptyDescription')"
        >
          <template #icon>
            <AppIcon size="lg"><path d="M3.5 8.4 6.5 11.4 12.5 5" /></AppIcon>
          </template>
        </EmptyState>
        <ul v-else class="rows">
          <li v-for="limit in activeRateLimits" :key="limit.provider" class="row">
            <div class="row__head">
              <strong>{{ limit.provider }}</strong>
              <span class="tag tag--error">
                {{ $t('settings.diagnostics.rateLimits.blocked') }}
              </span>
            </div>
            <dl class="meta">
              <div>
                <dt>{{ $t('settings.diagnostics.rateLimits.freeFrom') }}</dt>
                <dd :title="absoluteTime(limit.blocked_until)">
                  {{ relativeTime(limit.blocked_until) }}
                </dd>
              </div>
              <div>
                <dt>{{ $t('settings.diagnostics.rateLimits.streak') }}</dt>
                <dd>{{ limit.streak }}</dd>
              </div>
            </dl>
          </li>
        </ul>
      </article>
    </template>
  </section>
</template>

<style scoped>
.diagnostics {
  display: grid;
  gap: 24px;
}
header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 16px;
}
.support {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px 24px;
}
.actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
h3 {
  margin: 0;
  color: var(--text-primary);
}
h4 {
  margin: 0 0 10px;
  color: var(--text-primary);
  font-size: var(--font-size-md);
}
h5 {
  margin: 18px 0 8px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-semibold);
}
.hint {
  margin: 0 0 10px;
  color: var(--text-tertiary);
  font-size: var(--font-size-sm);
}
.status {
  margin: 0;
  color: var(--accent-red);
  font-size: var(--font-size-sm);
}
article {
  border-top: 1px solid var(--border-color-muted);
  padding-top: 18px;
}
.rows {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 10px;
}
.row {
  background: var(--bg-raised);
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  padding: 12px 14px;
  display: grid;
  gap: 8px;
}
.row__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
}
.row__error {
  margin: 0;
  color: var(--accent-red);
  font-size: var(--font-size-sm);
  overflow-wrap: anywhere;
}
.tag {
  flex: none;
  border-radius: var(--radius-full);
  padding: 2px 10px;
  background: var(--bg-hover);
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
}
.tag--error {
  background: color-mix(in srgb, var(--accent-red) 18%, var(--bg-raised));
  color: var(--accent-red);
}
.meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px 20px;
  margin: 0;
}
.meta div {
  display: grid;
  gap: 1px;
}
.meta dt {
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}
.meta dd {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
.table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--font-size-sm);
}
.table th {
  text-align: left;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-semibold);
  padding: 0 10px 6px 0;
}
.table td {
  color: var(--text-secondary);
  padding: 6px 10px 6px 0;
  border-top: 1px solid var(--border-color-muted);
}
.table .num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.row__subject {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mono {
  font-family: var(--font-mono);
  overflow-wrap: anywhere;
}
.histogram {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  gap: 6px;
}
.histogram__row {
  display: grid;
  grid-template-columns: minmax(160px, 1fr) 2fr auto;
  align-items: center;
  gap: 10px;
  font-size: var(--font-size-sm);
}
.histogram__label {
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.histogram__bar {
  display: block;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--accent-primary);
}
.histogram__count {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
}
</style>
