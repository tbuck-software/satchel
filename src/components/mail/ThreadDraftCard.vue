<script setup lang="ts">
import { computed, useId } from 'vue';
import type { Message } from '@/types/mail';
import AppButton from '@/components/ui/AppButton.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import CardButton from '@/components/ui/CardButton.vue';
import { draftPreviewText } from '@/utils/draftPreview';
import { formatMessageTimestamp } from '@/utils/listTime';

/*
 * An unsent draft at the end of a conversation: one object, and the only way
 * back into it. It is not a message — nothing was sent — so it does not take
 * the message layout (avatar, address line, rendered body) but reads as a
 * note lying on the thread: a dashed petrol frame, the label, who it is for,
 * when it was last saved and the first lines of what is written so far.
 * The whole card resumes it; the button says so for those who look for one.
 */
const props = withDefaults(defineProps<{
  message: Message;
  /** Who the draft is addressed to, by the name the thread knows them under. */
  recipient: string;
  /** The identity it would go out as, when that is worth saying. */
  identity?: string;
  identityColor?: string | null;
  /** A discard is already running. */
  busy?: boolean;
  keyboardFocused?: boolean;
}>(), {
  identity: '',
  identityColor: null,
  busy: false,
  keyboardFocused: false,
});

const emit = defineEmits<{
  edit: [];
  discard: [];
  focus: [];
}>();

const previewId = useId();
const preview = computed(() => draftPreviewText(props.message));
const savedAt = computed(() => formatMessageTimestamp(props.message.timestamp));
</script>

<template>
  <article
    class="draft-card"
    :class="{ 'draft-card--keyboard-focused': props.keyboardFocused }"
    :data-thread-message-id="props.message.id"
    tabindex="-1"
    @pointerdown="emit('focus')"
  >
    <CardButton
      class="draft-card__body"
      :aria-label="$t('thread.draft.open', { name: props.recipient })"
      :aria-describedby="previewId"
      @click="emit('edit')"
    >
      <span class="draft-card__head">
        <span class="draft-card__label">
          <AppIcon size="sm"><path d="M11.2 2.6l2.2 2.2L5.8 12.4 3 13l.6-2.8z" /><path d="M2.5 14.5h11" /></AppIcon>
          {{ $t('mail.message.draftNotSent') }}
        </span>
        <span class="draft-card__meta">
          <span class="draft-card__recipient">{{ $t('thread.draft.to', { name: props.recipient }) }}</span>
          <span v-if="props.identity" class="draft-card__identity">
            <span
              v-if="props.identityColor"
              class="draft-card__dot"
              :style="{ background: props.identityColor }"
              aria-hidden="true"
            />
            {{ $t('thread.reply.as', { identity: props.identity }) }}
          </span>
        </span>
        <span class="draft-card__saved">{{ $t('thread.draft.saved', { time: savedAt }) }}</span>
      </span>
      <span
        :id="previewId"
        class="draft-card__preview"
        :class="{ 'draft-card__preview--empty': !preview }"
      >{{ preview || $t('thread.draft.empty') }}</span>
    </CardButton>
    <!-- The row's empty stretch belongs to the card as well. -->
    <div class="draft-card__actions" @click.self="emit('edit')">
      <AppButton variant="primary" size="sm" @click="emit('edit')">
        <template #icon>
          <AppIcon><path d="M11.2 2.6l2.2 2.2L5.8 12.4 3 13l.6-2.8z" /><path d="M2.5 14.5h11" /></AppIcon>
        </template>
        {{ $t('thread.draft.continueEditing') }}
      </AppButton>
      <AppButton
        variant="ghost"
        size="sm"
        :disabled="props.busy"
        :title="$t('mail.message.discardDraft')"
        @click="emit('discard')"
      >
        <template #icon>
          <AppIcon><path d="M2.5 4h11M5.5 4V2.8A1.3 1.3 0 0 1 6.8 1.5h2.4a1.3 1.3 0 0 1 1.3 1.3V4" /><path d="M12.6 4v9a1.5 1.5 0 0 1-1.5 1.5H4.9A1.5 1.5 0 0 1 3.4 13V4" /></AppIcon>
        </template>
        {{ $t('common.discard') }}
      </AppButton>
    </div>
  </article>
</template>

<style scoped>
/* The frame is dashed, not drawn: the outline of something not yet there.
   It spans the reading column, the same edges the messages above keep. */
.draft-card {
  margin-top: var(--space-5);
  border: 1px dashed color-mix(in srgb, var(--accent-primary) 60%, var(--border-color));
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--accent-primary) 5%, var(--bg-surface));
  transition:
    background var(--duration-fast) var(--ease-out),
    border-color var(--duration-fast) var(--ease-out);
}

.draft-card + .draft-card {
  margin-top: var(--space-md);
}

.draft-card:hover {
  border-color: var(--accent-primary);
  background: color-mix(in srgb, var(--accent-primary) 8%, var(--bg-surface));
}

.draft-card:focus-visible,
.draft-card--keyboard-focused {
  outline: var(--border-focus);
  outline-offset: var(--focus-outline-offset);
}

.draft-card__body {
  flex-direction: column;
  gap: var(--space-sm);
  width: 100%;
  padding: var(--space-lg) var(--space-lg) var(--space-md);
  border-radius: var(--radius-md) var(--radius-md) 0 0;
}

.draft-card__head {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-xs) var(--space-md);
  font-size: var(--font-size-xs);
}

.draft-card__label {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1-5);
  color: var(--text-accent);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-semibold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
}

.draft-card__meta {
  display: inline-flex;
  align-items: center;
  gap: var(--space-md);
  min-width: 0;
  color: var(--text-secondary);
}

.draft-card__recipient {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-primary);
  font-weight: var(--weight-medium);
}

.draft-card__identity {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1-5);
  flex-shrink: 0;
}

.draft-card__dot {
  width: 7px;
  height: 7px;
  border-radius: var(--radius-full);
}

.draft-card__saved {
  margin-inline-start: auto;
  color: var(--text-secondary);
  white-space: nowrap;
}

/* Three lines are enough to recognise a draft; the composer has the rest. */
.draft-card__preview {
  display: -webkit-box;
  overflow: hidden;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  color: var(--text-primary);
  font-size: var(--font-size-md);
  line-height: 1.55;
  white-space: pre-line;
  overflow-wrap: anywhere;
}

.draft-card__preview--empty {
  color: var(--text-tertiary);
  font-style: italic;
}

.draft-card__actions {
  display: flex;
  align-items: center;
  gap: var(--space-sm);
  padding: 0 var(--space-lg) var(--space-lg);
  cursor: pointer;
}

/* A phone has a 16px gutter and a narrow line: the time goes under the label. */
@media (max-width: 480px) {
  .draft-card__body {
    padding: var(--space-md) var(--space-md) var(--space-sm);
  }

  .draft-card__actions {
    padding: 0 var(--space-md) var(--space-md);
  }

  .draft-card__saved {
    flex-basis: 100%;
    margin-inline-start: 0;
  }
}
</style>
