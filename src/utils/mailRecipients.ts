export function preferredExternalReplyAddress(
  replyTo: string | null | undefined,
  from: string | null | undefined,
  ownAddresses: Iterable<string>,
): string | null {
  const own = new Set(
    [...ownAddresses].map((address) => address.trim().toLowerCase()).filter(Boolean),
  );
  for (const candidate of [replyTo, from]) {
    const normalized = candidate?.trim() ?? '';
    if (normalized && !own.has(normalized.toLowerCase())) return normalized;
  }
  return null;
}
