import { computed, type Ref } from 'vue';
import {
  composerHtmlFromText,
  joinComposerTextHistory,
  splitComposerHtmlHistory,
  splitComposerTextHistory,
} from '@/utils/composerHistory';

/**
 * Drafts and send payloads retain one body, with signature metadata separate.
 * Editing presents two sections so the signature can sit between them without
 * becoming part of either editor or being duplicated by an identity change.
 */
export function useComposerBody(body: Ref<string>, bodyHtml: Ref<string | null>) {
  const text = computed(() => splitComposerTextHistory(body.value));
  const html = computed(() => splitComposerHtmlHistory(bodyHtml.value ?? composerHtmlFromText(body.value)));
  const writingText = computed({
    get: () => text.value.writing,
    set: (value: string) => { body.value = joinComposerTextHistory(value, text.value.history); },
  });
  const historyText = computed({
    get: () => text.value.history,
    set: (value: string) => { body.value = joinComposerTextHistory(text.value.writing, value); },
  });
  const writingHtml = computed({
    get: () => html.value.writing,
    set: (value: string) => { bodyHtml.value = value + html.value.history; },
  });
  const historyHtml = computed({
    get: () => html.value.history,
    set: (value: string) => { bodyHtml.value = html.value.writing + value; },
  });
  function removeHistory() {
    const writing = html.value.writing;
    body.value = text.value.writing;
    if (bodyHtml.value !== null) bodyHtml.value = writing;
  }
  return { writingText, writingHtml, historyText, historyHtml, removeHistory };
}
