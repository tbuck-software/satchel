import type { SweepRule } from '@/stores/sweepRules';

type Translate = (key: string, named?: Record<string, unknown>, plural?: number) => string;

/** One line for a recurring sender sweep: "Archive after 10 days", "Move to News". */
export function describeSweepRule(
  rule: Pick<SweepRule, 'action' | 'older_than_days' | 'keep_latest' | 'target_folder_id' | 'target_folder_name'>,
  t: Translate,
): string {
  const days = rule.older_than_days ?? 0;
  let base: string;
  if (rule.action === 'move') {
    base = rule.target_folder_id
      ? t('sweep.summary.move', { folder: rule.target_folder_name ?? '' })
      : t('sweep.summary.moveNeedsFolder');
  } else if (rule.action === 'trash') base = t('sweep.summary.trashAfter', { count: Math.max(1, days) }, Math.max(1, days));
  else base = days > 0 ? t('sweep.summary.archiveAfter', { count: days }, days) : t('sweep.summary.archive');
  return rule.keep_latest ? t('sweep.summary.keepingLatest', { rule: base }) : base;
}
