import type { Thread } from '@/types/mail';

export interface CiProject {
  forge: 'github' | 'gitlab';
  host: string;
  path: string;
}

const projectPath = /^[\w.-]+(?:\/[\w.-]+)+$/;
const githubRun = /^(?:PR\s+)?(?:workflow\s+)?run (?:failed|succeeded|passed|fixed|cancelled|canceled|skipped|timed out|completed):\s*\S/i;
const gitlabPipeline = /^(?:(?:failed|fixed|successful|success|passed|canceled|cancelled) pipeline\b|pipeline(?:\s+#?\d+)? (?:failed|fixed|succeeded|passed|canceled|cancelled)\b)/i;

/** CI events only: issues and reviews can themselves mention broken pipelines. */
export function detectCiProject(thread: Thread, subject: string): CiProject | null {
  // Forge conversation subjects end in an issue/MR number, including titles
  // that happen to look exactly like a CI notification.
  if (/\([#!]\d+\)\s*$/.test(subject)) return null;

  const sender = thread.participants[0]?.email.trim().toLowerCase() ?? '';
  const [local, senderHost = ''] = sender.split('@');
  if (!senderHost) return null;
  const githubSender = ['notifications', 'noreply', 'no-reply'].includes(local ?? '')
    && (senderHost === 'github.com' || senderHost.endsWith('.ghe.com'));
  const gitlabSender = local === 'gitlab'
    || (['notifications', 'noreply', 'no-reply'].includes(local ?? '')
      && (senderHost === 'gitlab.com' || /^gitlab[.-]/.test(senderHost)));
  const machineSender = gitlabSender || /^(?:notifications?|no[-_]?reply)$/.test(local ?? '');

  const bracket = subject.match(/^\[([^\]]+)\]\s*(.+)$/);
  if (githubSender && bracket && githubRun.test(bracket[2]!)) {
    const path = bracket[1]!.trim();
    if (projectPath.test(path) && path.split('/').length === 2) {
      return { forge: 'github', host: senderHost, path };
    }
  }

  // Without a repository prefix, require an actual Actions run URL.
  const githubEvent = githubSender && (
    githubRun.test(subject)
    || /^GitHub Actions:\s*.+\b(?:failed|succeeded|passed|cancelled|canceled)\b/i.test(subject)
  );
  const pipe = subject.match(/^(.+?)\s+\|\s+(.+)$/);
  const gitlabEvent = Boolean(pipe && gitlabPipeline.test(pipe[2]!));
  if (!githubEvent && !gitlabEvent) return null;
  if (!machineSender) return null;

  // The list already has a snippet. Do not load bodies or follow URLs to group.
  const linkedProjects = new Map<string, CiProject>();
  for (const raw of thread.snippet.match(/https?:\/\/[^\s<>"']+/gi) ?? []) {
    let url: URL;
    try { url = new URL(raw); } catch { continue; }
    if (url.username || url.password) continue;
    const host = url.host.toLowerCase();
    const github = url.pathname.match(/^\/([^/]+\/[^/]+)\/actions\/(?:runs|workflows)\//);
    const gitlab = url.pathname.match(/^\/(.+)\/-\/(?:pipelines|jobs)\//);
    if (githubEvent && host === senderHost && github && projectPath.test(github[1]!)) {
      const project: CiProject = { forge: 'github', host, path: github[1]! };
      linkedProjects.set(project.path.toLowerCase(), project);
    } else if (gitlabEvent && gitlab && projectPath.test(gitlab[1]!) && (
      gitlabSender || url.hostname === senderHost || url.hostname.endsWith(`.${senderHost}`)
    )) {
      const project: CiProject = { forge: 'gitlab', host, path: gitlab[1]! };
      linkedProjects.set(`${host}/${project.path}`, project);
    }
  }
  if (linkedProjects.size > 1) return null;
  if (linkedProjects.size === 1) return linkedProjects.values().next().value!;

  // Self-managed GitLab often uses a project display name in pipeline subjects.
  // A pipeline link, when present, supplies the full namespace instead.
  if (gitlabSender && gitlabEvent && pipe) {
    const path = pipe[1]!.trim();
    if (path) return { forge: 'gitlab', host: senderHost, path };
  }
  return null;
}
