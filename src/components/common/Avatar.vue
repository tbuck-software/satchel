<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { resolveSenderAvatar } from '@/services/senderAvatar';
import { useContactsStore } from '@/stores/contacts';

const props = defineProps<{
  name: string;
  email?: string | null;
  imageUrl?: string | null;
  size?: number;
}>();

const contacts = useContactsStore();
const sizeVal = computed(() => props.size ?? 32);
const imageFailed = ref(false);
const imageWidth = ref(0);
const contact = computed(() => contacts.findByEmail(props.email));
const avatar = computed(() =>
  resolveSenderAvatar({
    name: contact.value?.display_name || props.name,
    email: props.email,
    imageUrl: props.imageUrl || contact.value?.avatar_url,
    imageSource: props.imageUrl ? null : contact.value?.avatar_source,
    size: sizeVal.value * 2,
  }),
);
const showImage = computed(() => Boolean(avatar.value.imageUrl && !imageFailed.value));
/** A bare logo glyph, set on a light plate so a black mark survives a dark list. */
const onPlate = computed(() => showImage.value && avatar.value.source === 'favicon' && !avatar.value.tile);

watch(
  () => avatar.value.imageUrl,
  () => {
    imageFailed.value = false;
    imageWidth.value = 0;
  },
);
</script>

<template>
  <span
    :class="[
      'avatar',
      showImage ? 'avatar--image' : 'avatar--initials',
      showImage && avatar.source === 'provided' ? 'avatar--provided' : '',
      showImage && avatar.source === 'favicon' ? 'avatar--favicon' : '',
      onPlate ? 'avatar--plate' : '',
    ]"
    :style="{
      width: `${sizeVal}px`,
      height: `${sizeVal}px`,
      fontSize: `${Math.round(sizeVal * 0.38)}px`,
      '--avatar-color': avatar.color,
      backgroundColor: showImage
        ? onPlate ? 'var(--email-canvas)' : 'var(--bg-surface)'
        : 'color-mix(in srgb, var(--avatar-color) 16%, var(--bg-surface))',
    }"
  >
    <img
      v-if="showImage && avatar.imageUrl"
      :src="avatar.imageUrl"
      :alt="props.name"
      loading="lazy"
      decoding="async"
      :style="onPlate ? {
        width: `${Math.min(sizeVal * 0.78, imageWidth || sizeVal * 0.78)}px`,
        height: `${Math.min(sizeVal * 0.78, imageWidth || sizeVal * 0.78)}px`,
      } : undefined"
      @load="imageWidth = ($event.target as HTMLImageElement).naturalWidth"
      @error="imageFailed = true"
    />
    <span v-else class="avatar__initials">{{ avatar.initials }}</span>
  </span>
</template>

<style scoped>
.avatar {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  color: var(--text-on-accent);
  font-weight: var(--weight-semibold);
  letter-spacing: 0.02em;
  user-select: none;
  overflow: hidden;
}

.avatar .avatar__initials {
  color: color-mix(in srgb, var(--avatar-color) 45%, var(--text-primary));
}

.avatar img {
  display: block;
  width: 100%;
  height: 100%;
  border-radius: var(--radius-full);
}

.avatar--provided img {
  object-fit: cover;
}

.avatar--plate img {
  border-radius: 0;
  object-fit: contain;
}

/*
 * A logo's edge is a hairline, drawn over the image so a tile's own fill
 * cannot cover it. The plate alone had no edge in the light theme and a
 * glaring one in the dark.
 */
.avatar--favicon::after {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  box-shadow: inset 0 0 0 1px var(--border-color-muted);
  pointer-events: none;
}
</style>
