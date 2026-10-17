<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue';
import { useDismissable } from '@/composables/useDismissable';
import { useFocusTrap } from '@/composables/useFocusTrap';
import type { ExternalLinkAssessment } from '@/utils/externalLinks';
import { hasHoverCapablePointer } from '@/utils/pointerCapabilities';
import BottomSheet from '@/components/ui/BottomSheet.vue';
import ExternalLinkPrompt from './ExternalLinkPrompt.vue';

const props = defineProps<{
  assessment: ExternalLinkAssessment | null;
}>();

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

/**
 * The input device picks the container, not the window width: a finger gets
 * the bottom sheet on a tablet in landscape just as on a phone, while a
 * halved desktop window stays a pointer device and keeps the centred dialog.
 */
const sheetMode = !hasHoverCapablePointer();
const dialogRef = ref<HTMLElement | null>(null);
const open = computed(() => props.assessment !== null);

// The prompt inside puts the focus on its confirm button, in both containers.
// The trap only has to keep it there and hand it back.
const trap = useFocusTrap(dialogRef, {
  isOpen: () => open.value,
  moveFocusIn: () => false,
});

function onKeydown(event: KeyboardEvent) {
  if (!props.assessment) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    emit('cancel');
    return;
  }
  if (event.key !== 'Tab' || sheetMode) return;
  trap.onTab(event);
}

if (typeof window !== 'undefined') window.addEventListener('keydown', onKeydown);
onBeforeUnmount(() => window.removeEventListener('keydown', onKeydown));

// Back closes this before whatever it was opened from. In sheet mode the
// bottom sheet registers its own overlay entry; a second registration here
// would swallow one extra back press.
if (!sheetMode) {
  useDismissable(() => props.assessment !== null, () => emit('cancel'));
}
</script>

<template>
  <Teleport to="body">
    <!-- Touch-first layouts confirm at the bottom edge, where the thumb is. -->
    <BottomSheet
      v-if="sheetMode"
      :label="$t('mail.externalLink.sheetLabel')"
      :open="assessment !== null"
      @dismiss="emit('cancel')"
    >
      <section
        class="link-sheet"
        role="alertdialog"
        aria-modal="true"
        :aria-label="$t('mail.externalLink.dialogLabel')"
      >
        <ExternalLinkPrompt
          v-if="assessment"
          :key="assessment.href"
          :assessment="assessment"
          class="link-sheet-prompt"
          @confirm="emit('confirm')"
          @cancel="emit('cancel')"
        />
      </section>
    </BottomSheet>

    <Transition v-else name="link-dialog">
      <div
        v-if="assessment"
        class="link-dialog-backdrop"
        role="presentation"
        @pointerdown.self="emit('cancel')"
      >
        <section
          ref="dialogRef"
          class="link-dialog"
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="external-link-title"
          aria-describedby="external-link-description"
        >
          <div id="external-link-title" class="visually-grouped-title">
            {{ assessment.risks.length > 0
              ? $t('mail.externalLink.titleRisky')
              : $t('mail.externalLink.title') }}
          </div>
          <p id="external-link-description" class="visually-grouped-description">
            {{ assessment.risks.length > 0
              ? $t('mail.externalLink.descriptionRisky')
              : $t('mail.externalLink.description') }}
          </p>
          <ExternalLinkPrompt
            :key="assessment.href"
            :assessment="assessment"
            @confirm="emit('confirm')"
            @cancel="emit('cancel')"
          />
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.link-sheet {
  padding: 4px var(--space-lg) calc(var(--space-xl) + var(--satchel-safe-area-bottom, 0px));
}

.link-sheet-prompt {
  width: min(480px, calc(100vw - 2 * var(--space-lg)));
}

/*
 * The desktop alert dialog labels itself for assistive tech; the visible
 * heading lives inside the shared prompt, so the referenced ids sit here.
 */
.visually-grouped-title,
.visually-grouped-description {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

.link-dialog-backdrop {
  position: fixed;
  inset: 0;
  z-index: 1000;
  display: grid;
  place-items: center;
  padding: 24px;
  background: color-mix(in srgb, var(--bg-base) 66%, transparent);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
}

.link-dialog {
  position: relative;
  width: min(480px, 100%);
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: var(--radius-lg);
  background: color-mix(in srgb, var(--bg-surface) 98%, transparent);
  padding: 24px;
  box-shadow: 0 28px 80px rgba(0, 0, 0, 0.48);
}

.link-dialog-enter-active,
.link-dialog-leave-active { transition: opacity var(--duration-normal) var(--ease-out); }
.link-dialog-enter-active .link-dialog,
.link-dialog-leave-active .link-dialog { transition: transform var(--duration-normal) var(--ease-out), opacity var(--duration-normal) var(--ease-out); }
.link-dialog-enter-from,
.link-dialog-leave-to { opacity: 0; }
.link-dialog-enter-from .link-dialog,
.link-dialog-leave-to .link-dialog { opacity: 0; transform: translateY(8px) scale(0.985); }

@media (prefers-reduced-motion: reduce) {
  .link-dialog-enter-active,
  .link-dialog-leave-active,
  .link-dialog-enter-active .link-dialog,
  .link-dialog-leave-active .link-dialog { transition: none; }
}
</style>
