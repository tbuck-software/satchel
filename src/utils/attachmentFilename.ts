const RESERVED_DEVICE_NAME = /^(con|prn|aux|nul|conin\$|conout\$|com[0-9¹²³]|lpt[0-9¹²³])$/i;

/**
 * Reduce a name from mail to one path component, mirroring the backend's
 * `safe_attachment_name`: a directory part would preset the save dialog's folder.
 */
export function safeAttachmentFilename(name: string, fallback: string): string {
  const base = (name.split(/[/\\]/).pop() ?? '')
    .replace(/[:*?"<>|]/g, '_')
    .replace(/^[.\s]+/, '')
    .replace(/[.\s]+$/, '');
  if (!base) return fallback;
  const stem = base.split('.')[0]?.trimEnd() ?? '';
  return RESERVED_DEVICE_NAME.test(stem) ? `_${base}` : base;
}
