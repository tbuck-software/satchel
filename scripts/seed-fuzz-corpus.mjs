import { mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
for (const target of ['sanitizer', 'mime_body', 'ics', 'mbox_import']) {
  mkdirSync(resolve(root, 'fuzz/corpus', target), { recursive: true });
}
for (const name of readdirSync(resolve(root, 'src-tauri/tests/corpus')).filter((name) => name.endsWith('.eml'))) {
  const mail = readFileSync(resolve(root, 'src-tauri/tests/corpus', name));
  for (const target of ['mime_body', 'mbox_import', 'sanitizer', 'ics']) {
    writeFileSync(resolve(root, 'fuzz/corpus', target, name), mail);
  }
}
const tests = readFileSync(resolve(root, 'crates/satchel-core/src/email/sanitizer.rs'), 'utf8');
let index = 0;
// Keep the fuzz seeds aligned with the sanitizer's raw-string regression payloads.
for (const match of tests.matchAll(/r(#+)"([\s\S]*?)"\1/g)) {
  writeFileSync(resolve(root, 'fuzz/corpus/sanitizer', `regression-${index++}.html`), match[2]);
  if (match[2].includes('</sty@import;le>')) {
    for (const token of ['url()', 'expression(', 'behavior:', '-moz-binding']) {
      writeFileSync(resolve(root, 'fuzz/corpus/sanitizer', `regression-${index++}.html`), match[2].replace('@import;', token));
    }
  }
}
writeFileSync(resolve(root, 'fuzz/corpus/ics/invite.ics'),
  'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:fuzz-seed\r\nDTSTART:20261013T100000Z\r\nSUMMARY:Meeting\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n');
