import { describe, expect, it } from 'vitest';
import { crashReportForClipboard } from './crashReport';

describe('crashReportForClipboard', () => {
  it('omits a base64 trace while retaining its readable diagnosis', () => {
    const report = [
      '==== android native crash at now ====',
      'rss_kb: 2301568',
      'trace_printable_excerpt:',
      'Scudo ERROR: internal map failure (Out of memory)',
      'trace_encoding: base64',
      'trace:',
      'A'.repeat(300_000),
    ].join('\n');

    const copy = crashReportForClipboard(report);

    expect(copy).toContain('Scudo ERROR: internal map failure (Out of memory)');
    expect(copy).toContain('binary trace omitted');
    expect(copy).not.toContain('A'.repeat(1_000));
  });

  it('caps legacy reports that contain an unlabelled binary trace', () => {
    const copy = crashReportForClipboard('legacy trace\n' + 'x'.repeat(300_000));

    expect(copy.length).toBeLessThan(140_000);
    expect(copy).toContain('report truncated for clipboard');
  });
});
