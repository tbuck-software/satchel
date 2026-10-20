<script setup lang="ts">
import ModalShell from '@/components/ui/ModalShell.vue';
import AppButton from '@/components/ui/AppButton.vue';
import { useConfirm } from '@/composables/useConfirm';
import { useDismissableOnTop } from '@/composables/useDismissable';

/**
 * The one confirmation dialog. Mounted once in `App.vue`; `useConfirm().ask()`
 * fills it from anywhere.
 *
 * Every way out that is not the confirming button answers no, which is why
 * the back stack and the backdrop both go through `answer(false)`.
 */
const { state, answer } = useConfirm();

useDismissableOnTop(
  () => state.pending !== null,
  () => answer(false),
);
</script>

<template>
  <Teleport to="body">
    <ModalShell
      v-if="state.pending"
      :label="state.pending.title"
      width="420px"
      height="auto"
      @dismiss="answer(false)"
    >
      <div class="confirm">
        <h2 class="confirm__title">{{ state.pending.title }}</h2>
        <p v-if="state.pending.body" class="confirm__body">{{ state.pending.body }}</p>
        <p v-if="state.pending.detail" class="confirm__detail">{{ state.pending.detail }}</p>
        <footer class="confirm__actions">
          <AppButton variant="ghost" autofocus @click="answer(false)">
            {{ $t('common.cancel') }}
          </AppButton>
          <AppButton
            :variant="state.pending.tone === 'danger' ? 'danger' : 'primary'"
            @click="answer(true)"
          >
            {{ state.pending.confirmLabel }}
          </AppButton>
        </footer>
      </div>
    </ModalShell>
  </Teleport>
</template>

<style scoped>
.confirm {
  display: grid;
  gap: var(--space-md);
  padding: var(--space-xl);
}

.confirm__title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--weight-bold);
}

.confirm__body {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
}

.confirm__detail {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
  overflow-wrap: anywhere;
}

.confirm__actions {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-sm);
}
</style>
