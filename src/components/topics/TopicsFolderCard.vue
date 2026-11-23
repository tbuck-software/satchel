<script setup lang="ts">
import { computed } from 'vue';
import { Layers, Plus } from 'lucide';
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
        <AppIcon size="lg" :icon="Layers" />
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
        <AppIcon size="sm" :icon="Plus" />
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
