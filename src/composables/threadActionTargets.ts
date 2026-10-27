/** Selection owns mail actions; the open conversation is the fallback. */
export function threadActionTargets(selectedIds: readonly string[] = [], openId = ''): string[] {
  return selectedIds.length > 0 ? [...selectedIds] : openId ? [openId] : [];
}
