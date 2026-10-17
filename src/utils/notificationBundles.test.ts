import { describe, expect, it, vi } from 'vitest';
import type { Thread } from '@/types/mail';
import { splitSubjectPrefixes } from './composerText';
import { buildSimilarGroups, buildThreadDateSections, buildThreadEntries } from './threadList';

function notification(id: string, subject: string, sender = 'testflight_no_reply@email.apple.com'): Thread {
  const date = '2026-10-09T10:00:00Z';
  return {
    id, accountId: 'personal', subject, normalizedSubject: splitSubjectPrefixes(subject).base,
    participants: [{ name: sender, email: sender }],
    snippet: '', lastMessageAt: date, lastMessageAtMs: Date.parse(date),
    messageCount: 1, unreadCount: 1, hasAttachments: false,
    workflowState: 'inbox', priority: 0, mailboxNames: ['INBOX'], tags: [],
  };
}

const testBuild = (app: string, version: string, build: number, platform = 'iOS') =>
  `${app} ${version} (${build}) for ${platform} is now available to test.`;
const cronSender = 'no-reply@cron-job.org';
const cronJob = 'Example Map Maintenance Worker';

describe('repeated notification bundles', () => {
  it('bundles TestFlight versions and builds while keeping the latest subject and unread total', () => {
    const latest = notification('154', testBuild('Tau Agents', '0.7.40', 154));
    const threads = [
      latest,
      { ...notification('153', testBuild('Tau Agents', '0.7.40', 153)), unreadCount: 0 },
      notification('150', `Re: ${testBuild('Tau Agents', '0.7.39', 150)}`),
    ];
    const [group] = buildSimilarGroups(threads);
    expect(group).toMatchObject({ title: latest.subject, count: 3, unreadCount: 2 });
    expect(group?.threads.map((thread) => thread.id)).toEqual(['154', '153', '150']);
    expect(buildThreadEntries(threads, [group!])).toMatchObject([{ kind: 'group' }]);
  });

  it('retains app names, platforms, sender identities and accounts', () => {
    const first = notification('first', testBuild('Tau Agents', '0.7.40', 154));
    const variants = [
      notification('app', testBuild('Other App', '0.7.40', 155)),
      notification('platform', testBuild('Tau Agents', '0.7.40', 155, 'macOS')),
      { ...notification('account', testBuild('Tau Agents', '0.7.40', 155)), accountId: 'work' },
      { ...notification('developer', testBuild('Tau Agents', '0.7.40', 155)), participants: [{ name: 'Other developer', email: first.participants[0]!.email }] },
      notification('address', testBuild('Tau Agents', '0.7.40', 155), 'testflight@apple.com'),
    ];
    for (const variant of variants) expect(buildSimilarGroups([first, variant])).toEqual([]);
  });

  it('does not ignore app-name numbers or merge builds with invitations and expiry notices', () => {
    const threads = [
      notification('v1', testBuild('Editor 1.0', '2.0', 1)),
      notification('v2', testBuild('Editor 2.0', '2.0', 2)),
      notification('build', testBuild('Editor', '2.0', 3)),
      notification('invite', 'You are invited to test Editor.'),
      notification('expired', 'Editor 2.0 (3) for iOS has expired.'),
    ];
    expect(buildSimilarGroups(threads)).toEqual([]);
  });

  it('requires notification senders instead of grouping lookalike personal or issue subjects', () => {
    for (const sender of ['person@example.com', 'testflight_no_reply@email.apple.com.evil.test', 'notifications@github.com']) {
      expect(buildSimilarGroups([
        notification('one', testBuild('Tau Agents', '0.7.39', 150), sender),
        notification('two', testBuild('Tau Agents', '0.7.40', 154), sender),
      ])).toEqual([]);
    }
  });

  it.each([
    ['Cron-Job erfolgreich', 'Cron-Job fehlgeschlagen'],
    ['Cron job successful', 'Cron job failed'],
  ])('bundles job recovery with failures for %s and %s', (success, failure) => {
    const threads = [
      notification('fixed', `${success}: ${cronJob}`, cronSender),
      { ...notification('failed', `Re: ${failure}: ${cronJob}`, cronSender), messageCount: 2 },
      notification('other', `${failure}: Another Job`, cronSender),
    ];
    const [group] = buildSimilarGroups(threads);
    expect(group).toMatchObject({ title: threads[0]!.subject, count: 2, unreadCount: 2 });
    expect(group?.threads.map((thread) => thread.id)).toEqual(['fixed', 'failed']);
    expect(buildThreadEntries(threads, [group!])[1]).toMatchObject({ kind: 'thread', thread: { id: 'other' } });
  });

  it('keeps other job names, event types, senders and accounts separate', () => {
    const first = notification('first', `Cron-Job erfolgreich: ${cronJob}`, cronSender);
    const variants = [
      notification('job', 'Cron-Job fehlgeschlagen: Another Job', cronSender),
      notification('case', `Cron-Job fehlgeschlagen: ${cronJob.toLowerCase()}`, cronSender),
      notification('deleted', `Cron-Job gelöscht: ${cronJob}`, cronSender),
      notification('personal', `Cron-Job fehlgeschlagen: ${cronJob}`, 'person@example.com'),
      { ...notification('account', `Cron-Job fehlgeschlagen: ${cronJob}`, cronSender), accountId: 'work' },
    ];
    for (const variant of variants) expect(buildSimilarGroups([first, variant])).toEqual([]);
  });

  it('keeps complete cron commands, including script names, case, quoted whitespace and versions', () => {
    const command = (script: string) => `Cron <fixture-user@fixture-host> cd /var/www/${script} && php artisan schedule:run`;
    const first = notification('hook-one', command('hook'), 'fixture-user@mail.host.example');
    const threads = [first, notification('hook-two', first.subject, first.participants[0]!.email),
      notification('example-app', command('example-app'), first.participants[0]!.email)];
    expect(buildSimilarGroups(threads)[0]?.threads.map((thread) => thread.id)).toEqual(['hook-one', 'hook-two']);
    for (const [a, b] of [
      [command('hook'), command('Hook')],
      ["Cron <root@host> echo 'a  b'", "Cron <root@host> echo 'a b'"],
      ['Cron <root@host> echo Version 1.2.3', 'Cron <root@host> echo Version 1.2.4'],
    ]) {
      expect(buildSimilarGroups([notification('a', a!), notification('b', b!)])).toEqual([]);
    }
  });

  it('keeps bundle identity stable after a newer build is checked off', () => {
    const threads = [154, 153, 152].map((build) => notification(String(build), testBuild('Tau Agents', '0.7.40', build)));
    expect(buildSimilarGroups(threads.slice(1))[0]?.id).toBe(buildSimilarGroups(threads)[0]?.id);
  });

  it('keeps notification bundles within existing date sections', () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date('2026-10-10T12:00:00'));
    try {
      const now = new Date('2026-10-10T12:00:00');
      const today = new Date(now.getTime() - 60 * 60 * 1000);
      const yesterday = new Date(now.getTime() - 24 * 60 * 60 * 1000);
      const threads = [
        { ...notification('today', testBuild('Tau Agents', '0.7.40', 154)), lastMessageAt: today.toISOString(), lastMessageAtMs: today.getTime() },
        { ...notification('yesterday', testBuild('Tau Agents', '0.7.40', 153)), lastMessageAt: yesterday.toISOString(), lastMessageAtMs: yesterday.getTime() },
      ];
      expect(buildThreadDateSections(threads).flatMap((section) => section.similarGroups)).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});
