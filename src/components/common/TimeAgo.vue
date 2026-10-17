<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { formatDistanceToNow } from 'date-fns';
import { dateFnsLocale } from '@/i18n/dateFns';
import type { AppLocale } from '@/i18n/locale';

const props = defineProps<{
  date: string;
}>();

const { locale } = useI18n();

const value = computed(() =>
  formatDistanceToNow(new Date(props.date), {
    addSuffix: true,
    locale: dateFnsLocale(locale.value as AppLocale),
  }),
);
</script>

<template>
  <time :datetime="date" class="timeago">{{ value }}</time>
</template>

<style scoped>
.timeago {
  font-size: var(--font-size-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}
</style>
