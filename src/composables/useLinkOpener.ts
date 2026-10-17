import { ref } from 'vue';
import { useComposerStore } from '@/stores/composer';
import { useSettingsStore } from '@/stores/settings';
import { useUiStore } from '@/stores/ui';
import { useToast } from '@/composables/useToast';
import { t } from '@/i18n/runtime';
import { openExternalUrl } from '@/utils/openExternalUrl';
import { hasHoverCapablePointer } from '@/utils/pointerCapabilities';
import {
  assessExternalLink,
  parseMailtoLink,
  type ExternalLinkAssessment,
  type LinkAnchorRect,
  type LinkInteraction,
} from '@/utils/externalLinks';

const NO_ANCHOR: LinkAnchorRect = { left: 0, right: 0, top: 0, bottom: 0 };

export interface LinkOpenerOptions {
  /** Where a failed hand-off is shown; a toast unless the surface has its own. */
  onOpenFailed?: (href: string, anchor: LinkAnchorRect) => void;
}

/**
 * The one path from a link in mail content to the browser: mailto opens the
 * composer, everything else is assessed and, when it has to be, confirmed in
 * `ExternalLinkDialog` bound to `pending`.
 *
 * A trusted domain only skips the plain "leaving the app" question that touch
 * and single-pane layouts ask. A risk warning is asked every time.
 */
export function useLinkOpener(options: LinkOpenerOptions = {}) {
  const composer = useComposerStore();
  const settings = useSettingsStore();
  const ui = useUiStore();
  const toast = useToast();
  // Touch devices confirm every untrusted destination, whatever their width.
  const touchPrimary = !hasHoverCapablePointer();

  const pending = ref<ExternalLinkAssessment | null>(null);
  const pendingAnchor = ref<LinkAnchorRect | null>(null);

  function openFailed(href: string, anchor: LinkAnchorRect) {
    if (options.onOpenFailed) options.onOpenFailed(href, anchor);
    else toast.error(t('mail.link.browserFailed'));
  }

  async function openAssessed(assessment: ExternalLinkAssessment, anchor: LinkAnchorRect) {
    try {
      await openExternalUrl(assessment.href);
    } catch {
      openFailed(assessment.href, anchor);
    }
  }

  function needsPrompt(assessment: ExternalLinkAssessment): boolean {
    if (assessment.requiresConfirmation) return true;
    return (touchPrimary || ui.isSinglePane) && !settings.isTrustedLinkHost(assessment.hostname);
  }

  function open(link: LinkInteraction) {
    const mailto = parseMailtoLink(link.href);
    if (mailto) {
      pending.value = null;
      composer.openEmail(mailto.recipients, { subject: mailto.subject, body: mailto.body });
      return;
    }
    const assessment = assessExternalLink(link.href, link.label);
    if (!assessment.allowed) return;
    if (needsPrompt(assessment)) {
      pending.value = assessment;
      pendingAnchor.value = link.anchor;
      return;
    }
    void openAssessed(assessment, link.anchor);
  }

  function confirm() {
    const assessment = pending.value;
    if (!assessment) return;
    const anchor = pendingAnchor.value ?? NO_ANCHOR;
    cancel();
    void openAssessed(assessment, anchor);
  }

  function cancel() {
    pending.value = null;
    pendingAnchor.value = null;
  }

  return { pending, open, confirm, cancel };
}
