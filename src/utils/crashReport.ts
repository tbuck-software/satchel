const MAX_CLIPBOARD_REPORT_CHARS = 128 * 1024;

/** Keep clipboard reports below Android's Binder limit and omit binary payloads. */
export function crashReportForClipboard(raw: string): string {
  const binaryTraceMarker = '\ntrace_encoding: base64\ntrace:\n';
  const binaryTraceAt = raw.indexOf(binaryTraceMarker);
  const readable = binaryTraceAt >= 0
    ? `${raw.slice(0, binaryTraceAt + binaryTraceMarker.length)}(binary trace omitted; full trace remains in Satchel's log)\n`
    : raw;

  if (readable.length <= MAX_CLIPBOARD_REPORT_CHARS) return readable;
  return `${readable.slice(0, MAX_CLIPBOARD_REPORT_CHARS)}\n… report truncated for clipboard …\n`;
}
