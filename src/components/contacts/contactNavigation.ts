export type ContactsSection = 'all' | 'address-book' | 'frequent' | 'pinned';

export function resolveContactsSection(value: unknown): ContactsSection {
  const requested = Array.isArray(value) ? value[0] : value;
  if (
    requested === 'address-book'
    || requested === 'frequent'
    || requested === 'pinned'
  ) {
    return requested;
  }
  return 'all';
}
