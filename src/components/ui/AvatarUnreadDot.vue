<template>
  <span class="avatar-unread-dot" aria-hidden="true" />
</template>

<style scoped>
.avatar-unread-dot {
  position: absolute;
  top: 0;
  right: 0;
  z-index: 1;
  width: var(--space-sm);
  height: var(--space-sm);
  border-radius: var(--radius-full);
  background: var(--accent-primary);
  box-shadow: 0 0 0 var(--space-2xs) var(--avatar-dot-surface, var(--bg-list));
}
</style>
