import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const script = fileURLToPath(new URL('./verify-notarization-ticket.sh', import.meta.url));

/** @param {number} validFrom */
function verifyTicket(validFrom) {
  const root = mkdtempSync(join(tmpdir(), 'satchel-ticket-'));
  try {
    const tools = join(root, 'tools');
    mkdirSync(tools);
    writeFileSync(join(tools, 'xcrun'), `#!/bin/sh
test "$1" = stapler && test "$2" = validate && test "$3" = "$FIXTURE/Satchel Daily.app" || exit 9
count=$(cat "$FIXTURE/count" 2>/dev/null || printf 0)
count=$((count + 1))
printf '%s' "$count" > "$FIXTURE/count"
test "$count" -ge "$VALID_FROM" || exit 68
`, { mode: 0o700 });
    writeFileSync(join(tools, 'sleep'), '#!/bin/sh\nexit 0\n', { mode: 0o700 });
    const result = spawnSync('bash', [script, join(root, 'Satchel Daily.app')], {
      encoding: 'utf8',
      env: { ...process.env, PATH: `${tools}:${process.env.PATH}`, FIXTURE: root, VALID_FROM: String(validFrom) },
    });
    return { status: result.status, attempts: Number(readFileSync(join(root, 'count'), 'utf8')) };
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

describe.skipIf(process.platform === 'win32')('macOS ticket validation', () => {
  it('accepts a valid ticket immediately', () => {
    expect(verifyTicket(1)).toEqual({ status: 0, attempts: 1 });
  });

  it('retries a lost connection before accepting the ticket', () => {
    expect(verifyTicket(2)).toEqual({ status: 0, attempts: 2 });
  });

  it('retains the failure when no attempt verifies the ticket', () => {
    expect(verifyTicket(99)).toEqual({ status: 68, attempts: 3 });
  });
});
