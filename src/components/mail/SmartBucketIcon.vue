<script setup lang="ts">
import { computed } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import type { SmartInboxBucketKey, SmartInboxBucketSummary } from '@/stores/threads';

const props = withDefaults(
  defineProps<{
    bucketKey: SmartInboxBucketKey;
    tone: SmartInboxBucketSummary['tone'];
    size?: number;
    /**
     * `glyph` draws the icon alone in its hue, for collection rows in the
     * list: a tinted disc there read as one more avatar beside the people.
     * The tiles stay for headers, where the bundle is the page's subject.
     */
    shape?: 'rounded' | 'circle' | 'glyph';
  }>(),
  {
    size: 38,
    shape: 'rounded',
  },
);

const icon = computed(() => {
  if (props.bucketKey === 'priority') return 'priority';
  if (props.bucketKey === 'pinned') return 'pin';
  if (props.bucketKey === 'invitations') return 'calendar';
  if (props.bucketKey === 'notifications') return 'bell';
  if (props.bucketKey === 'newsletter') return 'newsletter';
  if (props.bucketKey === 'personal') return 'person';
  return 'mail';
});
</script>

<template>
  <span
    class="bucket-icon"
    :class="[`bucket-icon--${tone}`, `bucket-icon--${shape}`]"
    :style="{ width: `${size}px`, height: `${size}px` }"
    aria-hidden="true"
  >
    <AppIcon size="lg">
      <path v-if="icon === 'priority'" d="M9.2 1.8L4.4 8.4h3.4l-1 5.8 4.8-7H8.2l1-5.4z" />
      <template v-else-if="icon === 'pin'">
        <path d="M6.5 2.5h3l-.4 4.1 2.4 2.1v1H4.5v-1l2.4-2.1z" />
        <path d="M8 9.7v3.8" />
      </template>
      <template v-else-if="icon === 'calendar'">
        <rect x="2.5" y="3.5" width="11" height="10" rx="2" />
        <path d="M5 2v3M11 2v3M2.5 6.5h11" />
      </template>
      <template v-else-if="icon === 'bell'">
        <path d="M5 7.2a3 3 0 016 0c0 2 .8 2.8 1.4 3.4H3.6C4.2 10 5 9.2 5 7.2z" />
        <path d="M6.8 12.4a1.4 1.4 0 002.4 0" />
      </template>
      <template v-else-if="icon === 'newsletter'">
        <path d="M3 3.5h10v9H3z" />
        <path d="M5 6h6M5 8.5h6M5 11h3" />
      </template>
      <template v-else-if="icon === 'person'">
        <circle cx="8" cy="5.7" r="2.5" />
        <path d="M3.8 13a4.2 4.2 0 018.4 0" />
      </template>
      <template v-else>
        <rect x="2.8" y="4" width="10.4" height="8" rx="1.6" />
        <path d="M3.2 5.2L8 8.4l4.8-3.2" />
      </template>
    </AppIcon>
  </span>
</template>

<style scoped>
.bucket-icon {
  --bucket-hue: var(--text-accent);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  color: var(--bucket-hue);
}

.bucket-icon--rounded {
  border-radius: var(--radius-sm);
}

.bucket-icon--circle {
  border-radius: var(--radius-full);
}

.bucket-icon--priority,
.bucket-icon--pin,
.bucket-icon--newsletter {
  --bucket-hue: var(--accent-amber);
}

.bucket-icon--calendar {
  --bucket-hue: var(--accent-purple);
}

/* Petrol as a line, not a fill: --text-accent is the readable step of it. */
.bucket-icon--notification,
.bucket-icon--mail {
  --bucket-hue: var(--text-accent);
}

.bucket-icon--personal {
  --bucket-hue: var(--accent-green);
}

.bucket-icon--rounded,
.bucket-icon--circle {
  background: color-mix(in srgb, var(--bucket-hue) 15%, transparent);
}

.bucket-icon--rounded.bucket-icon--notification,
.bucket-icon--rounded.bucket-icon--mail,
.bucket-icon--circle.bucket-icon--notification,
.bucket-icon--circle.bucket-icon--mail {
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
  color: var(--accent-primary);
}
</style>
