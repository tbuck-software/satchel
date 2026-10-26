<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Topic } from '@/stores/topics';
import Avatar from '@/components/common/Avatar.vue';
import FolderPreviewCard from '@/components/mail/FolderPreviewCard.vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import IconButton from '@/components/ui/IconButton.vue';

const props = defineProps<{
  topics: Topic[];
  /** Names on the preview line; see FolderPreviewCard. */
  previewLimit?: number;
  targetKey?: string;
  keyboardFocused?: boolean;
}>();

const emit = defineEmits<{
  open: [];
  create: [];
}>();

const { t } = useI18n();

const unreadTotal = computed(() =>
  props.topics.reduce((sum, topic) => sum + topic.unread_count, 0),
);

// Chip visual, staged: user-set icon, else the newest linked sender's avatar,
// else the topic's own initials (Avatar renders them when no image resolves).
const chips = computed(() =>
  props.topics.map((topic) => ({
    id: topic.id,
    unread: topic.unread_count > 0,
    title: topic.unread_count > 0
      ? t('topics.card.unread', { name: topic.name, count: topic.unread_count })
      : topic.name,
    name: topic.name,
    icon: topic.icon,
    avatarName: topic.last_sender_name || topic.last_sender_email || topic.name,
    avatarEmail: topic.last_sender_email,
  })),
);
</script>

<template>
  <FolderPreviewCard
    :label="$t('topics.title')"
    :count="props.topics.length"
    :unread-count="unreadTotal"
    :chips="chips"
    :preview-limit="props.previewLimit"
    :target-key="props.targetKey"
    :keyboard-focused="props.keyboardFocused"
    :aria-label="$t('topics.card.label', { count: props.topics.length, unread: unreadTotal })"
    @click="emit('open')"
  >
    <template #icon>
      <span class="topics-glyph" aria-hidden="true">
        <AppIcon size="lg">
          <path d="M2 4.5C2 3.7 2.7 3 3.5 3H6L7.5 4.5H12.5C13.3 4.5 14 5.2 14 6V11.5C14 12.3 13.3 13 12.5 13H3.5C2.7 13 2 12.3 2 11.5V4.5Z" />
        </AppIcon>
      </span>
    </template>
    <template #chip="{ chip }">
      <span v-if="chip.icon" class="topics-chip-emoji" aria-hidden="true">{{ chip.icon }}</span>
      <Avatar v-else :name="chip.avatarName" :email="chip.avatarEmail" :size="18" />
      <span class="min-w-0 truncate">{{ chip.name }}</span>
    </template>
    <template #empty>{{ $t('topics.card.empty') }}</template>
    <template #actions>
      <IconButton size="sm" :title="$t('topics.card.new')" @click.stop="emit('create')">
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round">
          <path d="M8 3V13M3 8H13" />
        </svg>
      </IconButton>
    </template>
  </FolderPreviewCard>
</template>

<style scoped>
/* A glyph in the bundle language: hue on the line, no disc, so the folder
   does not read as one more sender beside the avatars. */
.topics-glyph {
  display: grid;
  place-items: center;
  width: 36px;
  height: 36px;
  color: var(--accent-purple);
  flex-shrink: 0;
}

.topics-chip-emoji {
  flex-shrink: 0;
  font-size: var(--font-size-sm);
  line-height: 1;
}
</style>
