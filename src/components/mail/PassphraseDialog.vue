<script setup lang="ts">
import { ref, watch } from 'vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { useDismissable } from '@/composables/useDismissable';
import { useI18n } from 'vue-i18n';

/**
 * Asks for the passphrase that unlocks a private key or certificate.
 *
 * This used to be `window.prompt`: an unstyled system box that shows the
 * passphrase in clear text, has no cancel semantics worth the name, and is not
 * guaranteed to exist in WKWebView at all. The shell brings the focus trap,
 * the scrim and `aria-modal` with it; Escape runs through
 * `ui.dismissTopmostOverlay()` like every other overlay.
 */
const props = defineProps<{
  /** Which material is locked, or `null` while nothing is being asked. */
  scheme: 'pgp' | 'smime' | null;
}>();

const emit = defineEmits<{ submit: [passphrase: string]; cancel: [] }>();

const { t } = useI18n();
const passphrase = ref('');

// Never carries over: the next question may be about a different key, and a
// passphrase left in a field is one an unlocked screen gives away.
watch(
  () => props.scheme,
  () => {
    passphrase.value = '';
  },
);

useDismissable(
  () => props.scheme !== null,
  () => emit('cancel'),
);

function title() {
  return props.scheme === 'smime' ? t('mail.smime.unlockTitle') : t('mail.pgp.unlockTitle');
}
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="scheme"
      :label="title()"
      width="420px"
      height="auto"
      :dismiss-on-backdrop="false"
      @dismiss="emit('cancel')"
    >
      <form class="passphrase" @submit.prevent="emit('submit', passphrase)">
        <h2 class="passphrase__title">{{ title() }}</h2>
        <p class="passphrase__description">
          {{ scheme === 'smime'
            ? $t('mail.smime.unlockTitleDescription')
            : $t('mail.pgp.unlockTitleDescription') }}
        </p>
        <label class="passphrase__field">
          <span>{{ scheme === 'smime'
            ? $t('mail.smime.passphraseLabel')
            : $t('mail.pgp.passphraseLabel') }}</span>
          <input
            v-model="passphrase"
            type="password"
            autofocus
            autocomplete="off"
            spellcheck="false"
          />
        </label>
        <footer class="passphrase__actions">
          <AppButton variant="ghost" type="button" @click="emit('cancel')">
            {{ $t('common.cancel') }}
          </AppButton>
          <AppButton variant="primary" type="submit">
            {{ scheme === 'smime' ? $t('mail.smime.unlock') : $t('mail.pgp.unlock') }}
          </AppButton>
        </footer>
      </form>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.passphrase {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-xl);
}

.passphrase__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
}

.passphrase__description {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.passphrase__field {
  display: grid;
  gap: var(--space-2xs);
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.passphrase__field input {
  width: 100%;
  padding: var(--space-xs) var(--space-sm);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.passphrase__field input:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.passphrase__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
