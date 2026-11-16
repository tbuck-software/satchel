<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';

/**
 * The quick reply under a chat. Enter sends at once (no Undo window, as in a
 * messenger); Shift+Enter starts a new line. Escape is the app's: it leaves
 * the chat (`useKeyboard`).
 */
const props = withDefaults(defineProps<{
  modelValue: string;
  recipient: string;
  sending?: boolean;
  disabled?: boolean;
}>(), { sending: false, disabled: false });

const emit = defineEmits<{ send: [text: string]; 'update:modelValue': [value: string] }>();
// A plain prop and event rather than `defineModel`: its runtime helper would
// land in the shared Vue chunk every window loads at start.
const text = computed({
  get: () => props.modelValue,
  set: (value: string) => emit('update:modelValue', value),
});

const field = ref<HTMLTextAreaElement | null>(null);
const MAX_ROWS_PX = 160;

function fit() {
  const element = field.value;
  if (!element) return;
  element.style.height = 'auto';
  element.style.height = `${Math.min(element.scrollHeight, MAX_ROWS_PX)}px`;
}

watch(text, () => void nextTick(fit));

function submit() {
  if (props.sending || props.disabled || !text.value.trim()) return;
  emit('send', text.value);
}

function onKeydown(event: KeyboardEvent) {
  // An IME composing a character uses Enter to commit it, not to send.
  if (event.key !== 'Enter' || event.shiftKey || event.isComposing) return;
  event.preventDefault();
  submit();
}

/** Keep the field in view when the on-screen keyboard pushes the page up. */
function onFocus() {
  window.setTimeout(() => field.value?.scrollIntoView({ block: 'nearest' }), 250);
}

defineExpose({ focus: () => field.value?.focus() });
</script>

<template>
  <form class="chat-composer" @submit.prevent="submit">
    <label class="chat-composer__field">
      <span class="sr-only">{{ $t('chats.composer.label') }}</span>
      <textarea
        ref="field"
        v-model="text"
        rows="1"
        enterkeyhint="send"
        :placeholder="$t('chats.composer.placeholder', { name: recipient })"
        :aria-describedby="'chat-composer-hint'"
        :disabled="disabled"
        data-chat-input
        @keydown="onKeydown"
        @focus="onFocus"
      />
    </label>
    <AppButton
      type="submit"
      variant="primary"
      class="chat-composer__send"
      :disabled="disabled || sending || !text.trim()"
      :shortcut="{ label: 'Enter', keys: ['Enter'], aria: 'Enter' }"
    >
      <AppIcon><path d="M14 2 2.5 7.5l4 1L8 13.5 14 2Z" /><path d="M6.5 8.5 14 2" /></AppIcon>
      {{ sending ? $t('chats.composer.sending') : $t('chats.composer.send') }}
    </AppButton>
    <p id="chat-composer-hint" class="chat-composer__hint">{{ $t('chats.composer.hint') }}</p>
  </form>
</template>

<style scoped>
.chat-composer {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: end;
  gap: var(--space-sm);
  padding: var(--space-sm) var(--space-md) calc(var(--space-sm) + env(safe-area-inset-bottom, 0px));
  border-top: 1px solid var(--border-color-muted);
  background: var(--bg-base);
}

.chat-composer__field {
  display: block;
  min-width: 0;
}

.chat-composer textarea {
  display: block;
  width: 100%;
  min-height: var(--touch-target);
  max-height: 160px;
  padding: var(--space-sm) var(--space-md);
  border: 1px solid var(--border-color);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-md);
  line-height: 1.4;
  resize: none;
}

.chat-composer textarea:focus-visible {
  border-color: var(--border-focus);
}

.chat-composer__send {
  min-height: var(--touch-target);
}

.chat-composer__hint {
  grid-column: 1 / -1;
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
}

/* A phone has no Shift+Enter to explain; the button is the way to send. */
@media (pointer: coarse) {
  .chat-composer__hint {
    display: none;
  }
}

.sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  white-space: nowrap;
}
</style>
