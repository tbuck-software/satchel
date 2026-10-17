<script setup lang="ts">
withDefaults(defineProps<{ compact?: boolean }>(), { compact: false });
</script>

<template>
  <div class="thread-skeleton" :class="{ 'thread-skeleton--compact': compact }" role="status" :aria-label="$t('thread.view.loading')">
    <div class="thread-skeleton__toolbar" aria-hidden="true">
      <i /><i /><i />
    </div>
    <div class="thread-skeleton__content" aria-hidden="true">
      <i class="thread-skeleton__title" />
      <section v-for="index in 2" :key="index" class="thread-skeleton__message">
        <i class="thread-skeleton__avatar" />
        <div class="thread-skeleton__lines">
          <i /><i /><i /><i />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.thread-skeleton {
  height: 100%;
  min-height: 280px;
  overflow: hidden;
  background: var(--bg-surface);
}

.thread-skeleton__toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 48px;
  padding: 0 16px;
  border-bottom: 1px solid var(--border-color-muted);
  background: var(--bg-surface);
  animation: thread-skeleton-pulse 1.2s ease-in-out infinite alternate;
}

.thread-skeleton__toolbar i {
  width: 28px;
  height: 28px;
  border-radius: var(--radius-md);
}

.thread-skeleton__content {
  display: grid;
  gap: 18px;
  width: min(768px, calc(100% - 40px));
  margin: 0 auto;
  padding: 28px 0;
  animation: thread-skeleton-pulse 1.2s ease-in-out infinite alternate;
}

.thread-skeleton__title {
  width: min(420px, 65%);
  height: 22px;
  border-radius: var(--radius-full);
}

.thread-skeleton__message {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  gap: 12px;
  min-height: 138px;
  padding: 18px;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
}

.thread-skeleton__avatar {
  width: 34px;
  height: 34px;
  border-radius: 50%;
}

.thread-skeleton__lines {
  display: grid;
  align-content: start;
  gap: 11px;
}

.thread-skeleton__lines i {
  width: 88%;
  height: 9px;
  border-radius: var(--radius-full);
}

.thread-skeleton__lines i:first-child { width: 36%; }
.thread-skeleton__lines i:nth-child(3) { width: 72%; }
.thread-skeleton__lines i:nth-child(4) { width: 54%; }

.thread-skeleton i {
  display: block;
  background: var(--bg-raised);
}

.thread-skeleton--compact .thread-skeleton__toolbar {
  display: none;
}

.thread-skeleton--compact .thread-skeleton__content {
  width: 100%;
  padding: 0;
}

.thread-skeleton--compact .thread-skeleton__title {
  display: none;
}

@keyframes thread-skeleton-pulse {
  to { opacity: 0.52; }
}

@media (prefers-reduced-motion: reduce) {
  .thread-skeleton__toolbar,
  .thread-skeleton__content { animation: none; }
}
</style>
