<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import CardButton from '@/components/ui/CardButton.vue';
import IconButton from '@/components/ui/IconButton.vue';
import DrawerButton from '@/components/layout/DrawerButton.vue';
import { useRouter } from 'vue-router';
import { useNewSenderActions } from '@/composables/useNewSenderActions';
import { useSettingsStore } from '@/stores/settings';
import { useThreadsStore, type NewSenderGroup } from '@/stores/threads';
import { isPersonalMailboxDomain, senderDomain } from '@/utils/senderDomains';
import Avatar from '@/components/common/Avatar.vue';
import { useI18n } from 'vue-i18n';

const router = useRouter();
const { t } = useI18n();
const settings = useSettingsStore();
const threads = useThreadsStore();
const newSenderActions = useNewSenderActions();

function accept(sender: NewSenderGroup) {
  newSenderActions.acceptSender(sender);
}

/** Domain-wide allow is only offered for organisation domains, never freemail. */
function allowableDomain(sender: NewSenderGroup): string {
  const domain = senderDomain(sender.senderEmail);
  return domain && !isPersonalMailboxDomain(domain) ? domain : '';
}

function acceptDomain(domain: string) {
  settings.acceptDomain(domain);
}

function acceptAll() {
  const senders = threads.newSenderStack?.senders ?? [];
  settings.acceptSenders(senders.map((sender) => sender.senderEmail));
}

function acceptExisting() {
  const senders = threads.newSenderStack?.senders ?? [];
  settings.completeSenderReviewSetup(senders.map((sender) => sender.senderEmail));
}

async function block(sender: NewSenderGroup) {
  await newSenderActions.blockSender(
    sender,
    t('newSenders.blockAndMoveConfirm', { email: sender.senderEmail }),
  );
}
</script>

<template>
  <section class="new-senders-view">
    <header>
      <AppButton @click="router.push({ name: 'inbox' })" variant="ghost" size="sm">{{ $t('newSenders.backToInbox') }}</AppButton>
      <div>
        <p>{{ $t('newSenders.eyebrow') }}</p>
        <DrawerButton /><h2>{{ $t('newSenders.title') }}</h2>
      </div>
      <span>{{ threads.newSenderStack?.count ?? 0 }}</span>
    </header>

    <div v-if="threads.newSenderStack?.senders.length" class="bulk-actions">
      <div>
        <strong v-if="!settings.senderReviewStartedAt">{{ $t('newSenders.bulk.existingTitle') }}</strong>
        <strong v-else>{{ $t('newSenders.bulk.manyTitle') }}</strong>
        <span v-if="!settings.senderReviewStartedAt">{{ $t('newSenders.bulk.existingHint') }}</span>
        <span v-else>{{ $t('newSenders.bulk.manyHint') }}</span>
      </div>
      <AppButton v-if="!settings.senderReviewStartedAt" class="primary" @click="acceptExisting" variant="primary" size="sm">
        {{ $t('newSenders.bulk.acceptExisting', threads.newSenderStack.count) }}
      </AppButton>
      <AppButton v-else class="primary" @click="acceptAll" variant="primary" size="sm">{{ $t('newSenders.bulk.acceptAll') }}</AppButton>
    </div>

    <div v-if="threads.newSenderStack?.senders.length" class="sender-list">
      <article v-for="sender in threads.newSenderStack.senders" :key="sender.id">
        <Avatar :name="sender.senderName" :email="sender.senderEmail" :size="42" />
        <CardButton class="sender-copy" @click="threads.selectThread(sender.topThreadId); router.push({ name: 'inbox' })">
          <strong>{{ sender.senderName }}</strong>
          <span>{{ sender.senderEmail }}</span>
          <span v-if="sender.spoofingText" class="spoofing">{{ sender.spoofingText }}</span>
          <span v-else>{{ sender.subject || sender.previewText }}</span>
        </CardButton>
        <span v-if="sender.count > 1" class="count">{{ sender.count }}</span>
        <div class="actions">
          <IconButton class="accept" :title="$t('newSenders.allow')" @click="accept(sender)">✓</IconButton>
          <IconButton
            v-if="allowableDomain(sender)"
            class="domain"
            :title="$t('newSenders.allowDomain', { domain: allowableDomain(sender) })"
            @click="acceptDomain(allowableDomain(sender))"
          >@</IconButton>
          <IconButton :title="$t('newSenders.block')" @click="block(sender)">×</IconButton>
        </div>
      </article>
    </div>
    <p v-else class="empty">{{ $t('newSenders.empty') }}</p>
  </section>
</template>

<style scoped>
.new-senders-view { height:100%; overflow:auto; padding:28px; color:var(--text-primary); }
header { max-width:760px; margin:0 auto 20px; display:grid; grid-template-columns:auto 1fr auto; gap:16px; align-items:center; }
header button { border:0; background:transparent; color:var(--accent-primary); cursor:pointer; }
header p { margin:0 0 3px; color:var(--accent-primary); font-size: var(--font-size-2xs); font-weight: var(--weight-bold); letter-spacing:.1em; text-transform:uppercase; }
h2 { margin:0; font-size:26px; }
header > span { min-width:34px; padding:5px 9px; border-radius: var(--radius-full); background:var(--bg-raised); text-align:center; font-weight: var(--weight-bold); }
.sender-list { max-width:760px; margin:0 auto; display:grid; gap:6px; }
.bulk-actions { max-width:760px; margin:0 auto 16px; display:flex; align-items:center; justify-content:space-between; gap:18px; padding:14px 16px; border:1px solid color-mix(in srgb,var(--accent-primary) 32%,var(--border-color-muted)); border-radius: var(--radius-md); background:color-mix(in srgb,var(--accent-primary) 10%,var(--bg-surface)); }
.bulk-actions div { min-width:0; display:grid; gap:3px; }
.bulk-actions span { color:var(--text-tertiary); font-size: var(--font-size-xs); }
.bulk-actions button { flex:none; border:0; border-radius: var(--radius-sm); padding:9px 12px; background:var(--accent-primary); color: var(--text-on-accent); font:inherit; font-size: var(--font-size-xs); font-weight: var(--weight-bold); cursor:pointer; }
article { display:grid; grid-template-columns:42px minmax(0,1fr) auto auto; gap:12px; align-items:center; padding:12px 14px; border:1px solid var(--border-color-muted); border-radius: var(--radius-md); background:var(--bg-surface); }
.sender-copy { min-width:0; display:grid; gap:2px; border:0; background:transparent; color:inherit; text-align:left; cursor:pointer; }
.sender-copy strong,.sender-copy span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.sender-copy span { color:var(--text-tertiary); font-size: var(--font-size-xs); }
.sender-copy span.spoofing { color:var(--accent-red); }
.count { color:var(--text-tertiary); font-size: var(--font-size-xs); }
.actions { display:flex; gap:6px; }
.actions button { width:34px; height:34px; border:0; border-radius:50%; background:var(--bg-overlay); color:var(--text-secondary); cursor:pointer; font-size:18px; }
.actions .accept { background:var(--accent-primary); color: var(--text-on-accent); }
.actions .domain { font-size: var(--font-size-lg); color:var(--accent-primary); background:color-mix(in srgb, var(--accent-primary) 12%, var(--bg-overlay)); }
.empty { margin:80px auto; text-align:center; color:var(--text-tertiary); }
@media (max-width: 620px) { .bulk-actions { align-items:stretch; flex-direction:column; } .bulk-actions button { width:100%; } }
</style>
