<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useSettingsStore } from '@/stores/settings';
import {
  DEFAULT_SWIPE_PREFERENCES,
  SWIPE_ACTION_OPTIONS,
  type SwipeActionId,
  type SwipeSlot,
} from '@/utils/swipeActions';
import AppButton from '@/components/ui/AppButton.vue';

const { t } = useI18n();
const settings = useSettingsStore();

const directions = computed<Array<{
  side: 'right' | 'left';
  title: string;
  hint: string;
  shortSlot: SwipeSlot;
  longSlot: SwipeSlot;
}>>(() => [
  {
    side: 'right',
    title: t('settings.swipes.directionRight'),
    hint: t('settings.swipes.directionRightHint'),
    shortSlot: 'rightShort',
    longSlot: 'rightLong',
  },
  {
    side: 'left',
    title: t('settings.swipes.directionLeft'),
    hint: t('settings.swipes.directionLeftHint'),
    shortSlot: 'leftShort',
    longSlot: 'leftLong',
  },
]);

const isDefault = computed(() =>
  Object.entries(DEFAULT_SWIPE_PREFERENCES).every(
    ([slot, action]) => settings.swipeActions[slot as SwipeSlot] === action,
  ),
);

function updateAction(slot: SwipeSlot, event: Event) {
  settings.setSwipeAction(slot, (event.target as HTMLSelectElement).value as SwipeActionId);
}
</script>

<template>
  <section class="swipe-settings">
    <header class="swipe-settings__header">
      <div>
        <p class="eyebrow">{{ $t('settings.swipes.eyebrow') }}</p>
        <h2>{{ $t('settings.swipes.title') }}</h2>
        <p class="description">{{ $t('settings.swipes.description') }}</p>
      </div>
      <AppButton variant="secondary" size="sm" :disabled="isDefault" @click="settings.resetSwipeActions()">
        {{ $t('common.default') }}
      </AppButton>
    </header>

    <div class="gesture-preview" aria-hidden="true">
      <span class="gesture-preview__action gesture-preview__action--read">
        <span class="gesture-preview__orb">○</span>
        {{ $t('settings.swipes.previewRead') }}
      </span>
      <span class="gesture-preview__mail">
        <span />
        <span />
        <span />
      </span>
      <span class="gesture-preview__action gesture-preview__action--pin">
        <span class="gesture-preview__orb">⌖</span>
        {{ $t('settings.swipes.previewPin') }}
      </span>
    </div>

    <div class="direction-grid">
      <article
        v-for="direction in directions"
        :key="direction.side"
        class="direction-card"
      >
        <div class="direction-card__title">
          <span class="direction-arrow" :class="`direction-arrow--${direction.side}`">→</span>
          <div>
            <h3>{{ direction.title }}</h3>
            <p>{{ direction.hint }}</p>
          </div>
        </div>

        <label class="action-field">
          <span>
            <strong>{{ $t('settings.swipes.short') }}</strong>
            <small>{{ $t('settings.swipes.shortHint') }}</small>
          </span>
          <select
            :value="settings.swipeActions[direction.shortSlot]"
            :data-swipe-setting="direction.shortSlot"
            :aria-label="$t('settings.swipes.ariaShort', { direction: direction.title })"
            @change="updateAction(direction.shortSlot, $event)"
          >
            <option
              v-for="option in SWIPE_ACTION_OPTIONS"
              :key="option.id"
              :value="option.id"
            >
              {{ $t(option.labelKey) }}
            </option>
          </select>
        </label>

        <label class="action-field">
          <span>
            <strong>{{ $t('settings.swipes.long') }}</strong>
            <small>{{ $t('settings.swipes.longHint') }}</small>
          </span>
          <select
            :value="settings.swipeActions[direction.longSlot]"
            :data-swipe-setting="direction.longSlot"
            :aria-label="$t('settings.swipes.ariaLong', { direction: direction.title })"
            @change="updateAction(direction.longSlot, $event)"
          >
            <option
              v-for="option in SWIPE_ACTION_OPTIONS"
              :key="option.id"
              :value="option.id"
            >
              {{ $t(option.labelKey) }}
            </option>
          </select>
        </label>
      </article>
    </div>

    <p class="device-note">{{ $t('settings.swipes.deviceNote') }}</p>
  </section>
</template>

<style scoped>
.swipe-settings {
  display: grid;
  gap: 22px;
}

.swipe-settings__header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 20px;
}

.eyebrow {
  margin: 0 0 6px;
  color: var(--accent-primary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: .11em;
  text-transform: uppercase;
}

h2,
h3,
p {
  margin: 0;
}

h2 {
  color: var(--text-primary);
  font-size: 21px;
  font-weight: var(--weight-bold);
  letter-spacing: -.02em;
}

.description {
  max-width: 540px;
  margin-top: 7px;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.55;
}

.gesture-preview {
  position: relative;
  display: grid;
  grid-template-columns: 1fr minmax(190px, 1.7fr) 1fr;
  min-height: 86px;
  overflow: hidden;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: var(--bg-inset);
}

.gesture-preview__action {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-on-accent);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-bold);
}

.gesture-preview__action--read { background: var(--swipe-blue); }
.gesture-preview__action--pin { background: var(--swipe-amber); }

.gesture-preview__orb {
  display: grid;
  place-items: center;
  width: 29px;
  height: 29px;
  border: 1.5px solid currentColor;
  border-radius: var(--radius-full);
  font-size: 18px;
  line-height: 1;
}

.gesture-preview__mail {
  z-index: 1;
  display: grid;
  align-content: center;
  gap: 8px;
  padding: 18px 24px;
  border-inline: 1px solid var(--border-color-muted);
  background: var(--bg-surface);
  box-shadow: 0 8px 22px rgba(0, 0, 0, .18);
}

.gesture-preview__mail span {
  height: 7px;
  border-radius: var(--radius-full);
  background: var(--bg-overlay);
}

.gesture-preview__mail span:nth-child(1) { width: 48%; background: var(--text-secondary); }
.gesture-preview__mail span:nth-child(2) { width: 82%; }
.gesture-preview__mail span:nth-child(3) { width: 66%; opacity: .65; }

.direction-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 14px;
}

.direction-card {
  overflow: hidden;
  border: 1px solid var(--border-color-muted);
  border-radius: var(--radius-md);
  background: color-mix(in srgb, var(--bg-surface) 78%, var(--bg-base));
}

.direction-card__title {
  display: flex;
  gap: 11px;
  padding: 15px 16px 13px;
  border-bottom: 1px solid var(--border-color-muted);
}

.direction-card__title h3 {
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  font-weight: var(--weight-bold);
}

.direction-card__title p {
  margin-top: 3px;
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  line-height: 1.4;
}

.direction-arrow {
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  flex: none;
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--accent-primary) 13%, var(--bg-raised));
  color: var(--accent-primary);
  font-size: 18px;
}

.direction-arrow--left { transform: rotate(180deg); }

.action-field {
  display: grid;
  grid-template-columns: minmax(82px, 1fr) minmax(132px, 1.35fr);
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
}

.action-field + .action-field {
  border-top: 1px solid var(--border-color-muted);
}

.action-field span {
  display: grid;
  gap: 2px;
}

.action-field strong {
  color: var(--text-secondary);
  font-size: var(--font-size-xs);
  font-weight: var(--weight-strong);
}

.action-field small {
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
}

.action-field select {
  min-width: 0;
  width: 100%;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-sm);
  padding: 8px 28px 8px 9px;
  background: var(--bg-raised);
  color: var(--text-primary);
  font: inherit;
  font-size: var(--font-size-2xs);
}

.action-field select:focus-visible {
  border-color: var(--accent-primary);
}

.device-note {
  color: var(--text-tertiary);
  font-size: var(--font-size-2xs);
  line-height: 1.5;
}

@media (max-width: 700px) {
  .direction-grid { grid-template-columns: 1fr; }
  .gesture-preview { grid-template-columns: .8fr 1.5fr .8fr; }
  .gesture-preview__action { gap: 5px; font-size: var(--font-size-2xs); }
  .gesture-preview__orb { width: 25px; height: 25px; }
}

@media (max-width: 430px) {
  .swipe-settings__header { align-items: center; }
  .gesture-preview__action { font-size: 0; }
  .action-field { grid-template-columns: 82px minmax(0, 1fr); }
}
</style>
