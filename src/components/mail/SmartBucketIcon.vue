<script setup lang="ts">
import { computed } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { Bell, Calendar, Mail, Newspaper, Pin, User, Zap } from 'lucide';
import type { IconNode } from 'lucide';
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

const BUCKET_ICONS: Partial<Record<SmartInboxBucketKey, IconNode>> = {
  priority: Zap,
  pinned: Pin,
  invitations: Calendar,
  notifications: Bell,
  newsletter: Newspaper,
  personal: User,
};

const icon = computed<IconNode>(() => BUCKET_ICONS[props.bucketKey] ?? Mail);
</script>

<template>
  <span
    class="bucket-icon"
    :class="[`bucket-icon--${tone}`, `bucket-icon--${shape}`]"
    :style="{ width: `${size}px`, height: `${size}px` }"
    aria-hidden="true"
  >
    <AppIcon size="lg" :icon="icon" />
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
