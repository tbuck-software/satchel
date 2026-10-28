<script setup lang="ts">
/**
 * An account's face: the picture the user chose, or its initials on its
 * colour. Unlike the sender avatar it never looks anything up — no contact
 * photo, no brand logo — so a Gmail account does not turn into the Gmail logo.
 */
import { computed, ref, watch } from 'vue';
import AppIcon from '@/components/ui/AppIcon.vue';
import { DEFAULT_ACCOUNT_COLOR, isAccountColor, wantsDarkInk } from '@/config/accountColors';
import { initialsFor } from '@/services/senderAvatar';

const props = withDefaults(
  defineProps<{
    name: string;
    color?: string | null;
    avatar?: string | null;
    size?: number;
    /** Dimmed with a moon: the account is resting on its schedule. */
    resting?: boolean;
  }>(),
  { color: null, avatar: null, size: 20, resting: false },
);

const imageFailed = ref(false);
watch(() => props.avatar, () => { imageFailed.value = false; });

const fill = computed(() => (props.color && isAccountColor(props.color) ? props.color : DEFAULT_ACCOUNT_COLOR));
const showImage = computed(() => Boolean(props.avatar?.startsWith('data:image/') && !imageFailed.value));
// One letter reads at sidebar size, two once there is room; below 14px a
// letter is a smudge, and the colour alone has to say whose it is.
const initials = computed(() => (props.size < 14 ? '' : initialsFor(props.name || '?').slice(0, props.size < 28 ? 1 : 2)));
</script>

<template>
  <span
    class="account-avatar"
    :class="{ 'account-avatar--resting': resting, 'account-avatar--dark-ink': wantsDarkInk(fill) }"
    :style="{
      width: `${size}px`,
      height: `${size}px`,
      fontSize: `${Math.round(size * 0.48)}px`,
      '--account-color': fill,
    }"
    aria-hidden="true"
  >
    <span class="account-avatar__face">
      <img v-if="showImage && avatar" :src="avatar" alt="" decoding="async" @error="imageFailed = true" />
      <span v-else>{{ initials }}</span>
    </span>
    <span v-if="resting" class="account-avatar__badge">
      <AppIcon size="sm"><path d="M13.5 9.5A5.8 5.8 0 0 1 6.5 2 6 6 0 1 0 13.5 9.5Z" /></AppIcon>
    </span>
  </span>
</template>

<style scoped>
.account-avatar {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

.account-avatar__face {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  overflow: hidden;
  border-radius: var(--radius-full);
  background: var(--account-color);
  color: var(--text-on-accent);
  font-weight: var(--weight-bold);
  line-height: 1;
  user-select: none;
}

/* The palette is bright; white initials on it fall under 3:1. */
.account-avatar--dark-ink .account-avatar__face {
  color: color-mix(in srgb, var(--account-color) 22%, var(--text-on-amber));
}

.account-avatar__face img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.account-avatar--resting .account-avatar__face {
  opacity: 0.45;
  filter: saturate(0.4);
}

.account-avatar__badge {
  position: absolute;
  right: -3px;
  bottom: -3px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 12px;
  height: 12px;
  border-radius: var(--radius-full);
  background: var(--avatar-ring, var(--bg-sidebar));
  color: var(--text-secondary);
}

.account-avatar__badge :deep(.app-icon) {
  width: 9px;
  height: 9px;
}
</style>
