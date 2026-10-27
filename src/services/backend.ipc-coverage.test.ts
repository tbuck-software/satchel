import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Guards against IPC commands that exist in Rust but are never reachable from
 * the UI — and against frontend calls to commands that do not exist.
 *
 * This test was added after a review found three fully implemented backend
 * subsystems with zero callers: FTS5 full-text search, server-side IMAP search
 * and the entire PGP command set (~1700 LOC). Nothing in the build reported it,
 * so the features silently did not exist for users.
 *
 * When you add a command, wire it up or add it to KNOWN_UNWIRED with a reason.
 */

/**
 * Commands intentionally not called from the frontend service layer.
 *
 * Every entry needs a reason that names the condition under which it goes away
 * again. `open_compose_window` was listed here as "called from the Rust tray"
 * while `services/backend/system.ts` had been invoking it since May — a dead
 * entry turns the guard off for a live command, so the tests below check the
 * allowlist itself.
 */
const KNOWN_UNWIRED: Record<string, string> = {
  set_thread_tags:
    'Tags left the mail UI (topics are the collections there) but stay in the backend and in rules. '
    + 'Goes when tags come back to the UI, or when the command is removed with them.',
};

/**
 * Exported helpers that legitimately have no UI caller. Same contract as
 * above: a reason that says what has to happen for the entry to disappear.
 * "no UI entry point yet" is not one — that is debt, not an exception.
 */
const UI_UNUSED_HELPERS_REASONS: Record<string, string> = {};

const UI_UNUSED_HELPERS = new Set<string>(Object.keys(UI_UNUSED_HELPERS_REASONS));

/**
 * Functions that carry `#[tauri::command]` without standing in
 * `generate_handler!`. The attribute alone generates serialisation code nobody
 * runs and advertises an IPC surface that does not exist — Tauri answers
 * "command not found" at runtime while every test stays green. `archive_threads`
 * was exactly that until ARCH-08; the notification quick action now calls
 * `archive_threads_inner` and the wrapper is gone. Same contract as above: a
 * reason that names the condition under which the entry disappears.
 */
const KNOWN_UNREGISTERED: Record<string, string> = {};

/**
 * Ceiling on the allowlist. It is empty, and that is the point: every backend
 * helper this tree ships has a path a person can walk to. Raising this is a
 * deliberate, reviewable edit, and the entry needs a reason next to it.
 */
const MAX_UI_UNUSED_HELPERS = 0;

function read(relativePath: string): string {
  return readFileSync(fileURLToPath(new URL(relativePath, import.meta.url)), 'utf8');
}

/**
 * Concatenated source of everything in src/ except the service layer itself.
 * `skip` drops one more file, which is how the tests below prove that a
 * helper's caller proof really runs through its UI entry point.
 */
function appSources(skip?: (path: string) => boolean): string {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const chunks: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      if (!/\.(ts|vue)$/.test(entry.name)) continue;
      if (entry.name.includes('.test.')) continue;
      if (full.includes(join('services', 'backend'))) continue;
      if (skip?.(full)) continue;
      chunks.push(readFileSync(full, 'utf8'));
    }
  };
  walk(root);
  return chunks.join('\n');
}

/** Command names registered in `tauri::generate_handler![...]`. */
function registeredCommands(): string[] {
  const source = read('../../src-tauri/src/lib.rs');
  const block = source.match(/invoke_handler\(tauri::generate_handler!\[([\s\S]*?)\]\)/);
  if (!block) throw new Error('could not locate generate_handler! block in lib.rs');
  return block[1]!
    .split(',')
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0 && !entry.startsWith('//'))
    .map((path) => path.split('::').pop() ?? '')
    .filter(Boolean);
}

/**
 * Every `#[tauri::command]`-annotated function under `src-tauri/src`.
 *
 * Further attributes may sit between the annotation and the signature
 * (`#[cfg(target_os = "macos")]`, `#[allow(...)]`), and the same name may be
 * annotated twice behind opposing `cfg`s — the result is deduplicated.
 */
function definedCommands(sources: string[]): string[] {
  const names = new Set<string>();
  for (const source of sources) {
    for (const match of stripComments(source).matchAll(
      /#\[tauri::command(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+([A-Za-z0-9_]+)/g,
    )) {
      names.add(match[1]!);
    }
  }
  return [...names].sort();
}

/** Every `.rs` file under `src-tauri/src`, read once. */
function backendSources(): string[] {
  const root = fileURLToPath(new URL('../../src-tauri/src', import.meta.url));
  const files: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith('.rs')) files.push(full);
    }
  };
  walk(root);
  return files.map((path) => readFileSync(path, 'utf8'));
}

/** Command names passed to `invoke(...)` anywhere in the frontend service layer. */
function invokedCommands(): Set<string> {
  const source = serviceLayerSources();
  const names = new Set<string>();
  for (const match of source.matchAll(/invoke\s*(?:<[^>]*>)?\s*\(\s*'([a-z0-9_]+)'/gi)) {
    names.add(match[1]!);
  }
  return names;
}

/** All files of the split service layer (src/services/backend/*.ts). */
function serviceLayerFiles(): string[] {
  const dir = fileURLToPath(new URL('./backend', import.meta.url));
  return readdirSync(dir)
    .filter((name) => name.endsWith('.ts'))
    .sort()
    .map((name) => join(dir, name));
}

function serviceLayerSources(): string {
  return serviceLayerFiles().map((path) => readFileSync(path, 'utf8')).join('\n');
}

/**
 * Removes `//` and block comments while leaving string literals intact, so a
 * helper that is only mentioned in a comment does not count as a caller.
 */
function stripComments(source: string): string {
  let out = '';
  let quote: string | null = null;
  let index = 0;
  while (index < source.length) {
    const char = source[index]!;
    const next = source[index + 1];
    if (quote !== null) {
      if (char === '\\') {
        out += char + (next ?? '');
        index += 2;
        continue;
      }
      if (char === quote) quote = null;
      out += char;
      index += 1;
      continue;
    }
    if (char === '"' || char === "'" || char === '`') {
      quote = char;
      out += char;
      index += 1;
      continue;
    }
    if (char === '/' && next === '/') {
      while (index < source.length && source[index] !== '\n') index += 1;
      continue;
    }
    if (char === '/' && next === '*') {
      index += 2;
      while (index < source.length && !(source[index] === '*' && source[index + 1] === '/')) {
        index += 1;
      }
      index += 2;
      continue;
    }
    out += char;
    index += 1;
  }
  return out;
}

/** Every `backend…` symbol the service layer exports, async or not. */
function exportedHelpers(serviceSource: string): string[] {
  return [
    ...stripComments(serviceSource).matchAll(
      /export\s+(?:async\s+)?(?:function|const)\s+(backend[A-Za-z0-9_]+)/g,
    ),
  ].map((match) => match[1]!);
}

/**
 * Exported helpers with no caller in `callerSource`. Matching is on word
 * boundaries: a substring test counts every helper whose name is a prefix of a
 * used one (`backendSetThreadStar` vs. `backendSetThreadStarred`) as wired.
 */
function unwiredHelpers(
  serviceSource: string,
  callerSource: string,
  allowlist: ReadonlySet<string> = UI_UNUSED_HELPERS,
): string[] {
  const callers = stripComments(callerSource);
  return exportedHelpers(serviceSource).filter(
    (name) => !allowlist.has(name) && !new RegExp(`\\b${name}\\b`).test(callers),
  );
}

/**
 * Allowlisted helpers that do have a caller — entries that outlived their
 * reason. The next reader takes this list as fact, so a stale entry is worse
 * than no list at all.
 */
function wiredAllowlistedHelpers(
  serviceSource: string,
  callerSource: string,
  allowlist: ReadonlySet<string>,
): string[] {
  const callers = stripComments(callerSource);
  const exported = new Set(exportedHelpers(serviceSource));
  return [...allowlist].filter(
    (name) => exported.has(name) && new RegExp(`\\b${name}\\b`).test(callers),
  );
}

/** Allowlisted commands the service layer does invoke after all. */
function wiredAllowlistedCommands(
  allowlist: Record<string, string>,
  invoked: ReadonlySet<string>,
): string[] {
  return Object.keys(allowlist).filter((name) => invoked.has(name));
}

function overCap(allowlist: ReadonlySet<string>, cap: number): boolean {
  return allowlist.size > cap;
}

/** Entries whose reason is too thin to say when they go away again. */
function reasonlessEntries(reasons: Record<string, string>, minLength: number): string[] {
  return Object.entries(reasons)
    .filter(([, reason]) => reason.trim().length < minLength)
    .map(([name]) => name);
}

describe('IPC coverage', () => {
  const registered = registeredCommands();
  const invoked = invokedCommands();

  it('parses a plausible command surface', () => {
    expect(registered.length).toBeGreaterThan(50);
    expect(invoked.size).toBeGreaterThan(50);
  });

  it('registers no command that the frontend cannot reach', () => {
    const unreachable = registered.filter(
      (name) => !invoked.has(name) && !(name in KNOWN_UNWIRED),
    );
    expect(
      unreachable,
      `These Tauri commands have no caller in services/backend.ts. Wire them up, `
        + `delete them, or add them to KNOWN_UNWIRED with a reason:\n  ${unreachable.join('\n  ')}`,
    ).toEqual([]);
  });

  /*
   * The two lists above compare *registered* commands with what the frontend
   * calls. A command that is annotated but never registered is invisible to
   * both: Tauri generates its serialisation glue, `lib.rs` never mentions it,
   * and an `invoke` for it fails at runtime with "command not found".
   */
  it('registers every #[tauri::command]', () => {
    const registeredSet = new Set(registered);
    const unregistered = definedCommands(backendSources()).filter(
      (name) => !registeredSet.has(name) && !(name in KNOWN_UNREGISTERED),
    );

    expect(
      unregistered,
      `These functions carry #[tauri::command] but are not in generate_handler!. `
        + `Register them, drop the attribute, or add them to KNOWN_UNREGISTERED `
        + `with a reason:\n  ${unregistered.join('\n  ')}`,
    ).toEqual([]);
  });

  it('reports an annotated command that never reached generate_handler!', () => {
    const source = [
      '#[tauri::command]',
      'pub async fn wired_up() -> CommandResult<()> { Ok(()) }',
      '',
      '#[tauri::command]',
      '#[cfg(target_os = "macos")]',
      'pub async fn forgotten(input: ThreadIdsInput) -> CommandResult<()> { Ok(()) }',
    ].join('\n');
    const registeredSet = new Set(['wired_up']);

    expect(definedCommands([source]).filter((name) => !registeredSet.has(name)))
      .toEqual(['forgotten']);
  });

  it('does not read a commented-out annotation as a command', () => {
    const source = [
      '// #[tauri::command]',
      '// pub async fn removed() {}',
      '#[tauri::command]',
      'fn kept() {}',
    ].join('\n');

    expect(definedCommands([source])).toEqual(['kept']);
  });

  it('keeps the KNOWN_UNREGISTERED allowlist honest', () => {
    const registeredSet = new Set(registered);
    const defined = new Set(definedCommands(backendSources()));
    const stale = Object.keys(KNOWN_UNREGISTERED).filter(
      (name) => registeredSet.has(name) || !defined.has(name),
    );

    expect(stale, 'allowlisted but registered again, or gone entirely').toEqual([]);
    expect(reasonlessEntries(KNOWN_UNREGISTERED, 10)).toEqual([]);
  });

  it('invokes no command that the backend does not register', () => {
    const registeredSet = new Set(registered);
    const missing = [...invoked].filter((name) => !registeredSet.has(name));
    expect(
      missing,
      `The frontend invokes commands that are not in generate_handler!:\n  ${missing.join('\n  ')}`,
    ).toEqual([]);
  });

  it('exports no backend helper that the UI never calls', () => {
    // A command can be "wired" here (the service layer calls invoke) and still
    // be unreachable because no component calls the exported helper. That is
    // exactly how the whole PGP key-management surface stayed dead: seven
    // bindings, zero .vue callers.
    const service = serviceLayerSources();
    expect(exportedHelpers(service).length).toBeGreaterThan(50);

    const unused = unwiredHelpers(service, appSources());

    expect(
      unused,
      `These backend helpers have no caller outside the service layer. Wire `
        + `them into the UI, delete them, or list them in UI_UNUSED_HELPERS:\n  ${unused.join('\n  ')}`,
    ).toEqual([]);
  });

  it('flags a helper whose name is a prefix of a used helper', () => {
    const service = `
      export async function backendSetThreadStar(id: string) {}
      export async function backendSetThreadStarred(id: string) {}
    `;
    const callers = `import { backendSetThreadStarred } from '@/services/backend';
      backendSetThreadStarred('t-1');`;

    expect(unwiredHelpers(service, callers)).toEqual(['backendSetThreadStar']);
  });

  it('does not count mentions inside comments as callers', () => {
    const service = 'export async function backendFoo() {}';

    expect(unwiredHelpers(service, '// backendFoo\nconst x = 1;')).toEqual(['backendFoo']);
    expect(unwiredHelpers(service, '/* backendFoo */\nconst x = 1;')).toEqual(['backendFoo']);
    expect(unwiredHelpers(service, 'backendFoo();')).toEqual([]);
  });

  it('covers non-async exports', () => {
    expect(exportedHelpers('export function backendThing(): boolean { return true; }')).toEqual([
      'backendThing',
    ]);
    expect(exportedHelpers('export const backendThing = () => true;')).toEqual(['backendThing']);
    expect(exportedHelpers(serviceLayerSources())).toContain('backendSupportsNativeCommands');
  });

  it('keeps the KNOWN_UNWIRED allowlist honest', () => {
    const registeredSet = new Set(registered);
    for (const [name, reason] of Object.entries(KNOWN_UNWIRED)) {
      expect(registeredSet.has(name), `${name} is allowlisted but no longer registered`).toBe(true);
      expect(reason.length, `${name} needs a reason`).toBeGreaterThan(10);
    }
    expect(
      wiredAllowlistedCommands(KNOWN_UNWIRED, invoked),
      'allowlisted but invoked by the service layer — drop the entry',
    ).toEqual([]);
  });

  it('flags an allowlisted command the service layer does invoke', () => {
    // The shape of the entry that went stale: the reason named the Rust tray,
    // services/backend/system.ts invoked it anyway.
    expect(
      wiredAllowlistedCommands({ open_compose_window: 'the tray calls it' }, new Set(['open_compose_window'])),
    ).toEqual(['open_compose_window']);
    expect(wiredAllowlistedCommands({ ghost_command: 'nobody calls it' }, new Set())).toEqual([]);
  });

  it('carries no helper on UI_UNUSED_HELPERS that the UI actually calls', () => {
    expect(
      wiredAllowlistedHelpers(serviceLayerSources(), appSources(), UI_UNUSED_HELPERS),
      'these are wired — take them off UI_UNUSED_HELPERS and lower the cap',
    ).toEqual([]);
  });

  it('flags an allowlisted helper that does have a caller', () => {
    const service = 'export async function backendFoo() {}';
    const allowlist = new Set(['backendFoo']);

    expect(wiredAllowlistedHelpers(service, 'backendFoo();', allowlist)).toEqual(['backendFoo']);
    expect(wiredAllowlistedHelpers(service, '// backendFoo', allowlist)).toEqual([]);
  });

  it('gives every allowlisted helper a reason that names its exit condition', () => {
    expect(reasonlessEntries(UI_UNUSED_HELPERS_REASONS, 20)).toEqual([]);
    // "no UI entry point yet" is the shape this rejects: it names no condition.
    expect(reasonlessEntries({ backendFoo: 'no UI yet' }, 20)).toEqual(['backendFoo']);
    expect(
      reasonlessEntries({ backendFoo: 'UX-99 wires it into the sender menu' }, 20),
    ).toEqual([]);
  });

  it('holds the unused-helper allowlist to its cap', () => {
    expect(
      UI_UNUSED_HELPERS.size,
      'a new exception needs the entry and a raised cap — both by hand',
    ).toBeLessThanOrEqual(MAX_UI_UNUSED_HELPERS);
    expect(overCap(UI_UNUSED_HELPERS, MAX_UI_UNUSED_HELPERS)).toBe(false);
  });

  it('rejects one entry more than the cap allows', () => {
    const oneTooMany = new Set([...UI_UNUSED_HELPERS, 'backendSomethingNew']);

    expect(oneTooMany.size).toBe(MAX_UI_UNUSED_HELPERS + 1);
    expect(overCap(oneTooMany, MAX_UI_UNUSED_HELPERS)).toBe(true);
  });

  /**
   * The caller proof has to run through the real path, not through a stray
   * import. Take the one file that calls a helper away and the guard reports
   * it as debt again — that is what makes a green run mean something.
   */
  // Four helpers, and each one walks the whole application tree to prove the
  // guard still reports it. That is over the 5 s default on an idle machine and
  // grows with the repo, so the budget is stated rather than inherited.
  it('reports a helper as unwired again once its only caller goes', { timeout: 30_000 }, () => {
    const service = serviceLayerSources();
    const soleCallers: Record<string, string> = {
      // Pane → helper.
      backendListProviderLabels: join('components', 'settings', 'ProviderFeaturesEditor.vue'),
      backendTestImapConnection: join('components', 'onboarding', 'OnboardingFlow.vue'),
      // Row → store → helper; the row is covered by the component tests.
      backendDeleteContact: join('stores', 'contacts.ts'),
      backendUpsertFollowUp: join('stores', 'followUps.ts'),
    };

    for (const [helper, caller] of Object.entries(soleCallers)) {
      expect(unwiredHelpers(service, appSources())).not.toContain(helper);
      expect(
        unwiredHelpers(service, appSources((path) => path.endsWith(caller))),
        `${helper} is reachable only through ${caller}`,
      ).toContain(helper);
    }
  });
});
