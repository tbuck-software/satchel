import { describe, expect, it } from 'vitest';

import { safeAttachmentFilename } from './attachmentFilename';

describe('safeAttachmentFilename', () => {
  it.each([
    ['/Users/x/Library/LaunchAgents/a.plist', 'a.plist'],
    ['..\\..\\Startup\\run.bat', 'run.bat'],
    ['Meeting 10:30.pdf', 'Meeting 10_30.pdf'],
    ['.hidden', 'hidden'],
    ['invoice.exe. ', 'invoice.exe'],
    ['nul.txt', '_nul.txt'],
    ['Rechnung.pdf', 'Rechnung.pdf'],
  ])('%s -> %s', (input, expected) => {
    expect(safeAttachmentFilename(input, 'attachment')).toBe(expected);
  });

  it.each(['', '..', 'a/..', 'dir/', '   '])('falls back for %j', (input) => {
    expect(safeAttachmentFilename(input, 'attachment')).toBe('attachment');
  });
});
