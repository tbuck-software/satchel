import { computed, ref, type Ref } from 'vue';
import { useI18n } from 'vue-i18n';
import type { Attachment } from '@/types/mail';
import { useAttachmentActions } from '@/composables/useAttachmentActions';
import { useToast } from '@/composables/useToast';
import { useSettingsStore } from '@/stores/settings';
import { matchesKeybinding } from '@/config/keybindings';

/**
 * What every attachment affordance does, once.
 *
 * Opening an attachment is four decisions — Satchel's viewer or the operating
 * system, which of them the click means, what Space does, and how a failure is
 * reported — and they were spelled out inside the chip. A second presentation
 * of the same file (the card in the reader) would otherwise have carried a
 * second copy, and the two would have drifted on the first change to the
 * open-behaviour setting.
 */
export function useAttachmentOpen(attachment: Ref<Attachment> | (() => Attachment)) {
  const target = typeof attachment === 'function' ? computed(attachment) : attachment;
  const { t } = useI18n();
  const toast = useToast();
  const actions = useAttachmentActions();
  const settings = useSettingsStore();

  const previewing = ref(false);
  const saving = ref(false);
  const opening = ref(false);
  const busy = computed(() => saving.value || opening.value);

  async function save() {
    if (busy.value) return;
    saving.value = true;
    try {
      await actions.downloadAndSaveAttachment(target.value.id, target.value.filename);
    } catch (error) {
      toast.error(t('mail.attachment.saveFailed', { error: String(error) }));
    } finally {
      saving.value = false;
    }
  }

  function openInSatchel() {
    if (!busy.value) previewing.value = true;
  }

  async function openWithSystem() {
    if (busy.value) return;
    opening.value = true;
    try {
      await actions.openAttachmentDefault(target.value.id);
    } catch (error) {
      toast.error(t('mail.attachment.openFailed', { error: String(error) }));
    } finally {
      opening.value = false;
    }
  }

  const systemFirst = computed(() => settings.attachmentOpenBehavior === 'system');

  function openPrimary() {
    if (systemFirst.value) void openWithSystem();
    else openInSatchel();
  }

  function openAlternate() {
    if (systemFirst.value) openInSatchel();
    else void openWithSystem();
  }

  async function openQuickLook() {
    if (busy.value) return;
    opening.value = true;
    try {
      const handled = await actions.toggleQuickLook(target.value.id);
      if (!handled) previewing.value = true;
    } catch (error) {
      toast.error(t('mail.attachment.quickLookFailed', { error: String(error) }));
    } finally {
      opening.value = false;
    }
  }

  function handlePreviewKey(event: KeyboardEvent) {
    if (!matchesKeybinding(event, settings.keybindings.previewAttachment)) return;
    event.preventDefault();
    event.stopPropagation();
    void openQuickLook();
  }

  const primaryTitle = computed(() => (systemFirst.value
    ? t('mail.attachment.openWithDefaultNamed', { filename: target.value.filename })
    : t('mail.attachment.viewInSatchelNamed', { filename: target.value.filename })));

  const alternateTitle = computed(() => (systemFirst.value
    ? t('mail.attachment.viewInSatchel')
    : t('mail.attachment.openWithDefault')));

  return {
    previewing,
    saving,
    opening,
    busy,
    systemFirst,
    primaryTitle,
    alternateTitle,
    save,
    openPrimary,
    openAlternate,
    handlePreviewKey,
  };
}
