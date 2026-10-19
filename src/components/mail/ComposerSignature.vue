<script setup lang="ts">
import AppButton from '@/components/ui/AppButton.vue';
import MenuItem from '@/components/ui/MenuItem.vue';
import { useDismissable } from '@/composables/useDismissable';
import { useUiStore } from '@/stores/ui';
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import type { BackendSignature } from '@/types/composer';
import { placePopoverAgainstAnchor, type PopoverPlacement } from '@/utils/popoverPlacement';

/**
 * Signature block at the bottom of the composer, Spark-style: the signature is
 * a visible, separate area of the message. The header carries a switcher
 * popover (which signature, which variant), the content itself is editable —
 * edits apply to this message only and are reported upward as an override.
 */
const props = defineProps<{
  signatures: BackendSignature[];
  selectedId: string | null;
  variant: 'auto' | 'long' | 'short' | 'none';
  loading: boolean;
  /** Ready-to-render HTML (override or resolved preview, cids hydrated). */
  displayHtml: string | null;
  /** Plain-text fallback / plain-text compose content. */
  displayText: string;
  overrideActive: boolean;
  plainText: boolean;
  disabled: boolean;
}>();

const emit = defineEmits<{
  select: [id: string | null];
  'variant-change': [variant: 'auto' | 'long' | 'short'];
  /** html is null in plain-text compose mode. */
  edit: [payload: { html: string | null; text: string }];
  reset: [];
}>();

const { t } = useI18n();

const menuOpen = ref(false);
const rootRef = ref<HTMLElement | null>(null);
const menuRef = ref<HTMLElement | null>(null);
const switchRef = ref<InstanceType<typeof AppButton> | null>(null);
const ui = useUiStore();
const editableRef = ref<HTMLElement | null>(null);
const editing = ref(false);
const menuStyle = ref<PopoverPlacement | null>(null);

const currentSignature = computed(
  () => props.signatures.find((signature) => signature.id === props.selectedId) ?? null,
);
const currentName = computed(() => currentSignature.value?.name ?? t('compose.signature.none'));

const variantOptions = computed(() => [
  { value: 'auto', label: t('compose.signature.variants.auto') },
  { value: 'long', label: t('compose.signature.variants.long') },
  { value: 'short', label: t('compose.signature.variants.short') },
] as const);

/**
 * The signature sits at the bottom of the composer's scroll container, so a
 * menu anchored inside it is clipped by that container long before it reaches
 * the viewport. It lives in a Teleport instead, placed against the button's
 * viewport rect.
 */
function positionMenu() {
  // A sheet is pinned to the bottom edge; there is nothing to place against.
  if (ui.isSinglePane) return;
  const anchor = switchRef.value?.$el.getBoundingClientRect();
  if (!anchor) return;
  menuStyle.value = placePopoverAgainstAnchor(anchor, {
    width: window.innerWidth,
    height: window.innerHeight,
  });
}

function toggleMenu() {
  // Place before opening, so the menu never paints at an unpositioned spot.
  if (!menuOpen.value) positionMenu();
  menuOpen.value = !menuOpen.value;
}

// Capture phase: the composer body scrolls, not the window.
watch(menuOpen, (open) => {
  if (open) {
    window.addEventListener('scroll', positionMenu, true);
    window.addEventListener('resize', positionMenu);
  } else {
    window.removeEventListener('scroll', positionMenu, true);
    window.removeEventListener('resize', positionMenu);
  }
});

function choose(id: string | null) {
  menuOpen.value = false;
  if (id !== props.selectedId) emit('select', id);
}

function chooseVariant(value: 'auto' | 'long' | 'short') {
  menuOpen.value = false;
  if (value !== props.variant) emit('variant-change', value);
}

function onDocumentPointerDown(event: PointerEvent) {
  if (!menuOpen.value) return;
  const target = event.target as Node;
  // The menu is teleported out of the block — check it separately.
  if (rootRef.value?.contains(target) || menuRef.value?.contains(target)) return;
  menuOpen.value = false;
}

function onDocumentKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && menuOpen.value) {
    event.stopPropagation();
    menuOpen.value = false;
  }
}

onMounted(() => {
  document.addEventListener('pointerdown', onDocumentPointerDown, true);
  document.addEventListener('keydown', onDocumentKeydown, true);
  syncEditable();
});

onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', onDocumentPointerDown, true);
  document.removeEventListener('keydown', onDocumentKeydown, true);
  window.removeEventListener('scroll', positionMenu, true);
  window.removeEventListener('resize', positionMenu);
});

/**
 * The contenteditable is uncontrolled while focused — rewriting innerHTML mid
 * edit would throw the caret to the start. Outside of an active edit it mirrors
 * displayHtml (signature switch, variant switch, reset).
 */
function syncEditable() {
  const el = editableRef.value;
  if (!el || editing.value) return;
  const next = props.displayHtml ?? '';
  if (el.innerHTML !== next) el.innerHTML = next;
}

watch(
  () => [props.displayHtml, props.selectedId, props.plainText, props.loading],
  () => nextTick(syncEditable),
);

function onHtmlInput() {
  const el = editableRef.value;
  if (!el) return;
  emit('edit', { html: el.innerHTML, text: el.innerText.replace(/\u00A0/g, ' ') });
}

function onTextInput(event: Event) {
  emit('edit', { html: null, text: (event.target as HTMLTextAreaElement).value });
}

// Back closes this before whatever it was opened from.
useDismissable(
  () => menuOpen.value,
  () => {
    menuOpen.value = false;
  },
);
</script>

<template>
  <div ref="rootRef" class="sig-block" data-composer-signature>
    <div class="sig-head">
      <span class="sig-title">{{ $t('compose.signature.title') }}</span>
      <template v-if="overrideActive">
        <span class="sig-edited" :title="$t('compose.signature.customizedTitle')">{{ $t('compose.signature.customized') }}</span>
        <AppButton
          class="sig-reset"
          :title="$t('compose.signature.resetTitle')"
          @click="emit('reset')"
          variant="link"
          size="sm"
        >
          {{ $t('common.reset') }}
        </AppButton>
      </template>
      <span class="sig-head-spacer" />
      <AppButton
        ref="switchRef"
        class="sig-switch-btn"
        :disabled="disabled"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        :title="$t('compose.signature.switch')"
        @click="toggleMenu"
        variant="ghost"
        size="sm"
      >
        <span class="sig-switch-name">{{ currentName }}</span>
        <svg
          :class="['sig-chevron', { 'sig-chevron--open': menuOpen }]"
          width="11"
          height="11"
          viewBox="0 0 12 12"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        ><path d="m3.5 4.5 2.5 2.5 2.5-2.5" /></svg>
      </AppButton>

      <Teleport to="body">
        <div
          v-if="menuOpen"
          ref="menuRef"
          class="sig-menu"
          :class="{ 'sig-menu--sheet': ui.isSinglePane }"
          :style="ui.isSinglePane ? undefined : menuStyle"
          role="menu"
          :aria-label="$t('compose.signature.switch')"
        >
          <MenuItem
            selection="radio"
            :label="$t('compose.signature.none')"
            :checked="selectedId === null"
            @select="choose(null)"
          />
          <MenuItem
            selection="radio"
            v-for="signature in signatures"
            :key="signature.id"
            :label="signature.name"
            :checked="signature.id === selectedId"
            @select="choose(signature.id)"
          />
          <template v-if="selectedId">
            <div class="sig-menu-sep" role="separator" />
            <div class="sig-menu-label">{{ $t('compose.signature.variant') }}</div>
            <MenuItem
            selection="radio"
              v-for="option in variantOptions"
              :key="option.value"
              :label="option.label"
              :checked="variant === option.value"
              @select="chooseVariant(option.value)"
          />
          </template>
        </div>
      </Teleport>
    </div>

    <p v-if="loading" class="sig-loading" aria-live="polite">{{ $t('compose.signature.loading') }}</p>
    <p v-else-if="!selectedId" class="sig-none">{{ $t('compose.signature.willSendWithout') }}</p>
    <template v-else>
      <textarea
        v-if="plainText"
        class="sig-text-edit"
        :value="displayText"
        :disabled="disabled"
        rows="3"
        :aria-label="$t('compose.signature.editAriaLabel')"
        spellcheck="true"
        @input="onTextInput"
      />
      <div
        v-else
        ref="editableRef"
        class="sig-html-edit"
        :contenteditable="!disabled"
        role="textbox"
        aria-multiline="true"
        :aria-label="$t('compose.signature.editAriaLabel')"
        spellcheck="true"
        @focus="editing = true"
        @blur="editing = false; syncEditable()"
        @input="onHtmlInput"
      />
    </template>
  </div>
</template>

<style scoped>
.sig-block {
  flex: none;
  margin: 14px 2px 2px;
  padding-top: 8px;
  border-top: 1px dashed var(--border-color-muted);
  cursor: default;
}

.sig-head {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

.sig-title {
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.07em;
  text-transform: uppercase;
}

.sig-edited {
  padding: 1px 6px;
  border-radius: var(--radius-full);
  background: color-mix(in srgb, var(--accent-primary) 14%, transparent);
  color: var(--accent-primary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-semibold);
}

.sig-head-spacer {
  flex: 1;
}

.sig-switch-btn { max-width:220px; }

.sig-switch-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sig-chevron {
  flex: none;
  transition: transform var(--duration-normal) var(--ease-out);
}

.sig-chevron--open {
  transform: rotate(180deg);
}

/* On a phone the menu rises from the bottom edge instead of hanging off a
   button halfway up the screen, and stops being positioned by script so
   nothing fights the pinning. */
.sig-menu--sheet {
  inset: auto 0 0 0;
  width: 100%;
  min-width: 0;
  max-width: none;
  max-height: 60vh;
  padding-right: var(--satchel-safe-area-right);
  padding-bottom: calc(5px + var(--satchel-safe-area-bottom));
  padding-left: var(--satchel-safe-area-left);
  border-radius: var(--radius-lg) var(--radius-lg) 0 0;
}

/* Placed from JS against the button's viewport rect — see positionMenu(). */
.sig-menu {
  position: fixed;
  z-index: 900;
  min-width: 240px;
  max-width: min(340px, calc(100vw - 60px));
  overflow: auto;
  overscroll-behavior: contain;
  display: grid;
  gap: 1px;
  padding: 5px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-md);
  background: var(--bg-overlay);
  box-shadow: var(--shadow-lg);
}

.sig-menu-sep {
  height: 1px;
  margin: 4px 2px;
  background: var(--border-color-muted);
}

.sig-menu-label {
  padding: 3px 9px 2px;
  color: var(--text-tertiary);
  font-size: var(--font-size-3xs);
  font-weight: var(--weight-bold);
  letter-spacing: 0.07em;
  text-transform: uppercase;
}

.sig-loading,
.sig-none {
  margin: 0;
  color: var(--text-tertiary);
  font-size: var(--font-size-xs);
}

.sig-html-edit,
.sig-text-edit {
  width: 100%;
  color: var(--text-secondary);
  font-size: var(--font-size-md);
  line-height: 1.55;
}

.sig-html-edit {
  cursor: text;
  outline: none;
  border-radius: var(--radius-sm);
}

.sig-html-edit:focus {
  box-shadow: var(--focus-ring);
}

.sig-html-edit :deep(p:first-child) {
  margin-top: 0;
}

.sig-html-edit :deep(p:last-child) {
  margin-bottom: 0;
}

.sig-html-edit :deep(img) {
  max-width: 100%;
  height: auto;
}

.sig-text-edit {
  min-height: 56px;
  padding: 6px 8px;
  border: 1px solid transparent;
  border-radius: var(--radius-sm);
  background: transparent;
  resize: vertical;
  font-family: var(--font-mono, ui-monospace, monospace);
}

.sig-text-edit:hover:not(:disabled),
.sig-text-edit:focus {
  border-color: var(--border-color);
}
</style>
