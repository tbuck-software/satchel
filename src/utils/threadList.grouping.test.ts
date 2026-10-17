import { describe, expect, it } from 'vitest';
import type { Thread } from '@/types/mail';
import { buildSimilarGroups, buildThreadDateSections, buildThreadEntries } from './threadList';
import { splitSubjectPrefixes } from './composerText';
import { detectCiProject } from './ciNotifications';

function mail(id: string, subject: string, name = 'CODE.EXAMPLE'): Thread {
  const date = '2026-10-07T10:00:00Z';
  return {
    id, accountId: 'work', subject, normalizedSubject: splitSubjectPrefixes(subject).base,
    participants: [{ name, email: 'notifications@code.example' }],
    snippet: '', lastMessageAt: date, lastMessageAtMs: Date.parse(date),
    messageCount: 1, unreadCount: 0, hasAttachments: false,
    workflowState: 'inbox', priority: 0, mailboxNames: ['INBOX'], tags: [],
  };
}

const pipelineSubject = 'Example CLI | Failed pipeline for feat/report-redesign';

describe('stable subject bundles', () => {
  it('uses the backend-normalized subject for localized replies and bundle display', () => {
    const first = { ...mail('first', 'Build #42 finished'), normalizedSubject: 'Build #42 finished' };
    const reply = { ...mail('reply', 'SV: Odp: Antw: Build #43 finished'), normalizedSubject: 'Build #43 finished' };
    const groups = buildSimilarGroups([first, reply]);
    expect(groups).toHaveLength(1);
    expect(groups[0]?.threads.map((thread) => thread.id)).toEqual(['first', 'reply']);
    expect(groups[0]?.title).not.toMatch(/SV:|Odp:|Antw:/);
  });

  it('keeps a different project and sender name out of a pipeline bundle', () => {
    const threads = [
      mail('merge-request', 'code.example | docs for deleting groups and repos (!350)', 'Jamie Taylor'),
      ...['d7ade544', '6d404a27', '15acb27a', '851220d8'].map((hash, index) =>
        mail(`pipeline-${index}`, `${pipelineSubject} | ${hash}`)),
    ];
    const groups = buildSimilarGroups(threads);
    expect(groups.map((group) => group.threads.map((thread) => thread.id))).toEqual([
      ['pipeline-0', 'pipeline-1', 'pipeline-2', 'pipeline-3'],
    ]);
    expect(buildThreadEntries(threads, groups)[0]).toMatchObject({
      kind: 'thread', thread: { id: 'merge-request' },
    });
  });

  it('keeps different branches, projects and issue numbers separate', () => {
    const subjects = [
      `${pipelineSubject} | d7ade544`,
      'Example CLI | Failed pipeline for main | 6d404a27',
      'Other CLI | Failed pipeline for feat/report-redesign | 15acb27a',
      'Example CLI | Review merge request (!350)',
      'Example CLI | Review merge request (!351)',
      'Invoice 123', 'Invoice 124',
      'Customer | d7ade544', 'Customer | 6d404a27',
    ];
    expect(buildSimilarGroups(subjects.map((subject, index) => mail(String(index), subject)))).toEqual([]);
  });

  it('requires the same sender address, display name and account even for equal subjects', () => {
    const first = mail('first', pipelineSubject);
    const variants = [
      { ...mail('other-account', pipelineSubject), accountId: 'personal' },
      mail('other-name', pipelineSubject, 'Jamie Taylor'),
      { ...mail('other-address', pipelineSubject), participants: [{ name: 'CODE.EXAMPLE', email: 'other@code.example' }] },
    ];
    for (const variant of variants) expect(buildSimilarGroups([first, variant])).toEqual([]);
  });

  it('ignores reply prefixes and explicit build counters but preserves event types', () => {
    const subjects = ['Build #42 finished', 'Re: Build #43 finished', 'Build #44 failed'];
    expect(buildSimilarGroups(subjects.map((subject, index) => mail(String(index), subject)))
      .map((group) => group.threads.map((thread) => thread.id))).toEqual([['0', '1']]);
  });

  it('keeps membership and identity stable when an intervening email is checked off', () => {
    const threads = [
      mail('pipeline-new', pipelineSubject),
      mail('other-project', 'Satchel | Failed pipeline for main'),
      mail('pipeline-middle', pipelineSubject),
      mail('pipeline-old', pipelineSubject),
    ];
    const before = buildSimilarGroups(threads);
    const after = buildSimilarGroups(threads.filter((thread) => thread.id !== 'other-project'));
    expect(before.map((group) => group.threads.map((thread) => thread.id))).toEqual([
      ['pipeline-new', 'pipeline-middle', 'pipeline-old'],
    ]);
    expect(after).toEqual(before);
    const withoutLead = buildSimilarGroups(threads.slice(1));
    expect(withoutLead[0]?.id).toBe(before[0]?.id);
  });
});

function ciMail(id: string, subject: string, email = 'notifications@github.com'): Thread {
  return { ...mail(id, subject, 'Alex Morgan'), participants: [{ name: 'Alex Morgan', email }] };
}

describe('CI project bundles', () => {
  it('bundles the screenshot by repository across workflows, branches, hashes and sender names', () => {
    const threads = [
      ciMail('satchel-ci', '[tbuck-software/satchel] Run failed: CI - main (b4c37e4)'),
      ciMail('tau-main', '[example-owner/tau] Run failed: Performance gates - main (ee0d01d)'),
      { ...ciMail('satchel-security', '[tbuck-software/satchel] Run failed: Security audit - main (b4c37e4)'),
        participants: [{ name: 'GitHub', email: 'noreply@github.com' }], unreadCount: 2 },
      ciMail('tau-pr', '[example-owner/tau] PR run failed: Performance gates - feat(ui): a sidebar (123abcd)'),
      ciMail('satchel-fixed', 'Re: [tbuck-software/satchel] Run succeeded: CI - main (a7fa0b2)'),
      ciMail('other', '[example-owner/example-map] PR run failed: CI - Fix search (abcdef0)'),
    ];
    const groups = buildSimilarGroups(threads);
    expect(groups.map((group) => ({
      project: group.ciProject, ids: group.threads.map((thread) => thread.id), unread: group.unreadCount,
    }))).toEqual([
      { project: 'tbuck-software/satchel', ids: ['satchel-ci', 'satchel-security', 'satchel-fixed'], unread: 2 },
      { project: 'example-owner/tau', ids: ['tau-main', 'tau-pr'], unread: 0 },
    ]);
    expect(buildThreadEntries(threads, groups).map((entry) => entry.kind)).toEqual(['group', 'group', 'thread']);
    const withoutLead = buildSimilarGroups(threads.slice(1));
    expect(withoutLead.find((group) => group.ciProject === 'tbuck-software/satchel')?.id).toBe(groups[0]?.id);
  });

  it('keeps issues, reviews, merge conflicts and account security out even when they mention CI', () => {
    const subjects = [
      '[owner/repo] New issue: Pipeline failed (#123)',
      '[owner/repo] Run failed: CI - main (#124)',
      '[owner/repo] Review requested: Fix CI (#125)',
      '[owner/repo] Merge conflict (#126)',
      'GitHub security alert: New sign-in',
      'CLI | Failed pipeline for main (!350)',
      'CLI | Review merge request (!351)',
    ];
    for (const subject of subjects) {
      const thread = ciMail('issue', subject, subject.startsWith('CLI') ? 'gitlab@code.example' : undefined);
      thread.snippet = 'Pipeline failed: https://github.com/owner/repo/actions/runs/42';
      expect(detectCiProject(thread, subject)).toBeNull();
    }
    const threads = [
      ciMail('failed', '[owner/repo] Run failed: CI - main (abcdef0)'),
      ciMail('issue', subjects[1]!),
      ciMail('fixed', '[owner/repo] Run succeeded: CI - main (abcdef1)'),
    ];
    expect(buildSimilarGroups(threads)[0]?.threads.map((thread) => thread.id)).toEqual(['failed', 'fixed']);
    expect(buildThreadEntries(threads, buildSimilarGroups(threads))[1]).toMatchObject({ kind: 'thread', thread: { id: 'issue' } });
  });

  it('requires forge notification senders for subject-only matches', () => {
    const subject = '[owner/repo] Run failed: CI - main (abcdef0)';
    for (const email of ['person@example.com', 'person@github.com', 'notifications@github.com.evil.test']) {
      expect(detectCiProject(ciMail('personal', subject, email), subject)).toBeNull();
    }
  });

  it('bundles failed and fixed GitLab pipelines on a self-managed instance', () => {
    const threads = [
      ciMail('failed', `${pipelineSubject} | d7ade544`, 'gitlab@code.example'),
      ciMail('fixed', 'Example CLI | Fixed pipeline for main | 6d404a27', 'gitlab@code.example'),
      ciMail('other', 'Other CLI | Successful pipeline for main | 15acb27a', 'gitlab@code.example'),
    ];
    expect(buildSimilarGroups(threads)[0]).toMatchObject({
      ciProject: 'Example CLI', count: 2, threads: [{ id: 'failed' }, { id: 'fixed' }],
    });
  });

  it('uses GitLab pipeline links to separate namespaces and hosts with the same display name', () => {
    const threads = [
      { ...ciMail('failed', 'CLI | Failed pipeline for main', 'notifications@code.example'),
        snippet: 'See https://code.example/team/subgroup/cli/-/pipelines/42' },
      { ...ciMail('fixed', 'CLI | Fixed pipeline for feat/report', 'notifications@code.example'),
        snippet: 'See https://code.example/team/subgroup/cli/-/pipelines/43' },
      { ...ciMail('other-namespace', 'CLI | Failed pipeline for main', 'notifications@code.example'),
        snippet: 'See https://code.example/other/cli/-/pipelines/44' },
      { ...ciMail('other-host', 'CLI | Fixed pipeline for main', 'gitlab@other.example'),
        snippet: 'See https://other.example/team/subgroup/cli/-/pipelines/45' },
    ];
    expect(buildSimilarGroups(threads)).toMatchObject([
      { ciProject: 'team/subgroup/cli', threads: [{ id: 'failed' }, { id: 'fixed' }] },
    ]);
    expect(buildSimilarGroups(threads)).toHaveLength(1);
  });

  it('recognizes a GitHub Actions project from a run link when the subject has no repository', () => {
    const subjects = ['GitHub Actions: deploy.yml failed', 'GitHub Actions: checks.yml succeeded'];
    const threads = subjects.map((subject, index) => ({
      ...ciMail(String(index), subject), snippet: `See https://github.com/owner/repo/actions/runs/${index}`,
    }));
    expect(buildSimilarGroups(threads)[0]).toMatchObject({ ciProject: 'owner/repo', count: 2 });
    expect(detectCiProject({ ...threads[0]!, snippet: 'https://github.com/owner/repo/issues/42' }, subjects[0]!)).toBeNull();
  });

  it('does not choose one project when multiple pipeline links disagree', () => {
    const subject = 'CLI | Failed pipeline for main';
    expect(detectCiProject({
      ...ciMail('ambiguous', subject, 'gitlab@code.example'),
      snippet: 'https://code.example/team/cli/-/pipelines/1 https://code.example/other/cli/-/pipelines/2',
    }, subject)).toBeNull();
  });

  it('keeps accounts and calendar days separate, including inside a broad date section', () => {
    const first = ciMail('first', '[owner/repo] Run failed: CI - main (abcdef0)');
    const variants = [
      { ...first, id: 'other-account', accountId: 'personal' },
      { ...first, id: 'other-day', lastMessageAt: '2026-10-06T10:00:00Z', lastMessageAtMs: Date.parse('2026-10-06T10:00:00Z') },
    ];
    for (const variant of variants) expect(buildSimilarGroups([first, variant], 'month-2026-10')).toEqual([]);
    const sections = buildThreadDateSections([first, variants[1]!]);
    expect(sections.flatMap((section) => section.similarGroups)).toEqual([]);
  });
});
