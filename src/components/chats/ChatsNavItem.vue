<script setup lang="ts">
import { computed } from 'vue';
import { MessageSquare } from 'lucide';
import { useRoute, useRouter } from 'vue-router';
import NavItem from '@/components/ui/NavItem.vue';
import AppIcon from '@/components/ui/AppIcon.vue';

/** The sidebar row of Labs → Chats; it mounts only while the flag is on. */
const emit = defineEmits<{ navigate: [] }>();
const route = useRoute();
const router = useRouter();
const active = computed(() => route.name === 'chats');

function open() {
  emit('navigate');
  void router.push({ name: 'chats' });
}
</script>

<template>
  <NavItem
    :label="$t('chats.title')"
    :active="active"
    :aria-current="active ? 'page' : undefined"
    data-nav="chats"
    @click="open"
  >
    <template #icon>
      <AppIcon :icon="MessageSquare" />
    </template>
  </NavItem>
</template>
