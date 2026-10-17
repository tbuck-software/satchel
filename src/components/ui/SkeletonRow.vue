<script setup lang="ts">
withDefaults(
  defineProps<{
    count?: number;
    height?: string;
    lines?: number;
  }>(),
  { count: 6, height: '60px', lines: 2 },
);
</script>

<template>
  <div class="skeleton-list">
    <div
      v-for="i in count"
      :key="i"
      class="skeleton-row"
      :style="{ height }"
    >
      <span class="skeleton-row__avatar" />
      <span class="skeleton-row__content">
        <span
          v-for="line in lines"
          :key="line"
          class="skeleton-row__line"
          :class="`skeleton-row__line--${Math.min(line, 3)}`"
        />
      </span>
      <span class="skeleton-row__meta" />
    </div>
  </div>
</template>

<style scoped>
.skeleton-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
  padding: var(--space-md);
  animation: skeleton-pulse 1.2s ease-in-out infinite alternate;
}
.skeleton-row {
  display: flex;
  align-items: center;
  gap: var(--space-md);
  min-height: 52px;
  padding: var(--space-2-5) var(--space-md);
  box-sizing: border-box;
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-raised) 62%, transparent);
}
.skeleton-row__avatar {
  width: 36px;
  height: 36px;
  flex: 0 0 36px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}
.skeleton-row__content {
  display: grid;
  flex: 1 1 auto;
  min-width: 0;
  gap: var(--space-1-5);
}
.skeleton-row__line,
.skeleton-row__meta {
  display: block;
  height: 8px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}
.skeleton-row__line--1 { width: min(46%, 190px); }
.skeleton-row__line--2 { width: min(78%, 330px); }
.skeleton-row__line--3 { width: min(62%, 260px); }
.skeleton-row__meta {
  width: 38px;
  flex: 0 0 38px;
  align-self: flex-start;
  margin-top: var(--space-xs);
}
@keyframes skeleton-pulse {
  to { opacity: 0.56; }
}

@media (prefers-reduced-motion: reduce) {
  .skeleton-list { animation: none; }
}
</style>
