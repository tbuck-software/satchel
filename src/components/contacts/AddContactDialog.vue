<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue';
import { useI18n } from 'vue-i18n';
import { useContactsStore } from '@/stores/contacts';
import { useUiStore } from '@/stores/ui';
import { useToast } from '@/composables/useToast';
import { backendErrorLabel } from '@/utils/backendErrorLabels';
import ModalShell from '@/components/ui/ModalShell.vue';
import PaneHeader from '@/components/ui/PaneHeader.vue';
import AppButton from '@/components/ui/AppButton.vue';

const emit = defineEmits<{ dismiss: [] }>();
const { t } = useI18n();
const contacts = useContactsStore();
const ui = useUiStore();
const toast = useToast();
const email = ref('');
const name = ref('');
const saving = ref(false);

function dismiss() {
  if (!saving.value) emit('dismiss');
}

onBeforeUnmount(ui.registerOverlay({ isOpen: () => true, dismiss }));

async function save() {
  if (saving.value) return;
  saving.value = true;
  try {
    await contacts.saveProfile({ email: email.value, displayName: name.value, notes: '' });
    toast.success(t('contacts.add.saved'));
    emit('dismiss');
  } catch (cause) {
    toast.error(`${t('contacts.add.failed')} ${backendErrorLabel(cause)}`);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <ModalShell :label="t('contacts.add.title')" width="440px" height="auto" @dismiss="dismiss">
    <PaneHeader :title="t('contacts.add.title')" no-menu />
    <form class="contact-form" @submit.prevent="save">
      <label>{{ t('contacts.add.email') }}<input v-model="email" type="email" autocomplete="email" required :disabled="saving" /></label>
      <label>{{ t('contacts.add.name') }}<input v-model="name" autocomplete="name" :disabled="saving" /></label>
      <div class="contact-form__actions">
        <AppButton variant="ghost" :disabled="saving" @click="dismiss">{{ t('common.cancel') }}</AppButton>
        <AppButton variant="primary" type="submit" :disabled="saving">{{ t('common.save') }}</AppButton>
      </div>
    </form>
  </ModalShell>
</template>

<style scoped>
.contact-form {
  display: grid;
  gap: var(--space-lg);
  padding: var(--space-lg);
}
label {
  display: grid;
  gap: var(--space-sm);
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}
input {
  width: 100%;
  padding: var(--space-2-5) var(--space-md);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-raised);
  color: var(--text-primary);
  font: inherit;
}
.contact-form__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
