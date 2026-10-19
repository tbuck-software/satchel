<script setup lang="ts">
import { ref, watch } from 'vue';
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { usePrompt } from '@/composables/usePrompt';
import { useDismissableOnTop } from '@/composables/useDismissable';

/**
 * The one text-input dialog. Mounted once in `App.vue`; `usePrompt().ask()`
 * fills it from anywhere.
 *
 * Every way out that is not the submit button answers `null`. The backdrop
 * does not dismiss: a stray click must not throw away what was typed.
 */
const { state, answer } = usePrompt();
const value = ref('');

// Each question starts from its own initial value; a passphrase typed for the
// previous one must not carry over.
watch(
  () => state.pending,
  (pending) => {
    value.value = pending?.initialValue ?? '';
  },
  { immediate: true },
);

useDismissableOnTop(
  () => state.pending !== null,
  () => answer(null),
);
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="state.pending"
      :label="state.pending.title"
      width="420px"
      height="auto"
      :dismiss-on-backdrop="false"
      @dismiss="answer(null)"
    >
      <form class="prompt" @submit.prevent="answer(value)">
        <h2 class="prompt__title">{{ state.pending.title }}</h2>
        <p v-if="state.pending.body" class="prompt__body">{{ state.pending.body }}</p>
        <label class="prompt__field">
          <span>{{ state.pending.label }}</span>
          <input
            v-model="value"
            :type="state.pending.inputType ?? 'text'"
            autofocus
            autocomplete="off"
            spellcheck="false"
          />
        </label>
        <footer class="prompt__actions">
          <AppButton variant="ghost" type="button" @click="answer(null)">
            {{ $t('common.cancel') }}
          </AppButton>
          <AppButton variant="primary" type="submit">
            {{ state.pending.confirmLabel }}
          </AppButton>
        </footer>
      </form>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.prompt {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-xl);
}

.prompt__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
}

.prompt__body {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.prompt__field {
  display: grid;
  gap: var(--space-2xs);
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
}

.prompt__field input {
  width: 100%;
  padding: var(--space-xs) var(--space-sm);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  background: var(--bg-base);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-sm);
}

.prompt__field input:focus-visible {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.prompt__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
