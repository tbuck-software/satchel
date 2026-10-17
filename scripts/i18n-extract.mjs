#!/usr/bin/env node
/**
 * Finds UI text that is still hard-coded instead of going through `t()`.
 *
 *   node scripts/i18n-extract.mjs           every finding, grouped by file
 *   node scripts/i18n-extract.mjs --german  only what the heuristic reads as German
 *   node scripts/i18n-extract.mjs --check   exit 1 when German text is left over
 *
 * German is detected by umlauts/ß or a wordlist; false positives belong in
 * scripts/i18n-allowlist.json.
 */
import { readFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';
import { parse } from 'vue/compiler-sfc';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const allowlistPath = path.join(root, 'scripts', 'i18n-allowlist.json');

// These files render text outside Vue or form the structured-error boundary.
// Keep the list explicit: scanning every Rust string would mistake protocols,
// SQL, fixtures and provider diagnostics for product copy.
const NATIVE_PRESENTATION_FILES = new Set([
  'src-tauri/src/commands/error.rs',
  'src-tauri/src/commands/folders.rs',
  'src-tauri/src/commands/inline_cache.rs',
  'src-tauri/src/commands/message_body.rs',
  'src-tauri/src/commands/notifications.rs',
  'src-tauri/src/commands/oauth.rs',
  'src-tauri/src/commands/onboarding.rs',
  'src-tauri/src/commands/outbox.rs',
  'src-tauri/src/commands/tray.rs',
  'src-tauri/src/commands/ui_strings.rs',
  'src-tauri/src/events/bus.rs',
  'src-tauri/src/sync/maintenance.rs',
  'src-tauri/src/sync/mailbox_sync.rs',
  'src-tauri/src/sync/message_ingest.rs',
  'src-tauri/src/sync/outbox.rs',
  'src-tauri/src/sync/remote_mutations.rs',
  'src-tauri/src/sync/workflow_executor.rs',
  'src-tauri/src/utils/mac_dock_menu.rs',
  'src-tauri/src/utils/mac_notify.rs',
  'src-tauri/gen/android/app/src/main/java/de/tbuck/satchel/background/NotificationActionWork.kt',
  'src-tauri/gen/android/app/src/main/java/de/tbuck/satchel/background/SatchelMessagingService.kt',
  'src-tauri/gen/android/app/src/main/java/de/tbuck/satchel/background/SatchelNotifications.kt',
]);

const TEXT_ATTRS = new Set([
  'title',
  'aria-label',
  'aria-description',
  'aria-placeholder',
  'aria-roledescription',
  'aria-valuetext',
  'placeholder',
  'alt',
  'label',
  'subtitle',
  'description',
  'hint',
  'empty-label',
  'confirm-label',
  'cancel-label',
]);

/*
 * Whole-word markers, restricted to words that are unambiguously German in a
 * UI string. Ambiguous ones ("in", "man", "war", "hat", "die" as an English
 * verb) stay out so the lint does not fire on English copy.
 */
const GERMAN_WORDS = [
  'abbrechen', 'ablegen', 'abrufen', 'aktualisieren', 'alle', 'alles', 'anhang',
  'anhänge', 'ansehen', 'anzeigen', 'archivieren', 'auch', 'auf', 'aus',
  'auswählen', 'bearbeiten', 'beim', 'bereits', 'bestätigen', 'bitte', 'dabei',
  'dann', 'darstellung', 'das', 'dass', 'dein', 'deine', 'deinem', 'deinen',
  'deiner', 'dem', 'den', 'der', 'des', 'dich', 'diese', 'diesem', 'diesen',
  'dieser', 'dieses', 'durch', 'eine', 'einen', 'einer', 'eines',
  'einstellungen', 'entfernen', 'erneut', 'erstellen', 'fehler', 'für',
  'geladen', 'gelesen', 'gesendet', 'gespeichert', 'hinzufügen', 'ihre',
  'immer', 'jetzt', 'kann', 'keine', 'keinen', 'konnte', 'konten', 'konto',
  'laden', 'lädt', 'löschen', 'mehr', 'mit', 'muss', 'müssen', 'nach',
  'nachricht', 'nachrichten', 'neue', 'neuen', 'nicht', 'noch', 'nur', 'oder',
  'ohne', 'schließen', 'schon', 'seine', 'senden', 'sich', 'sind', 'soll',
  'sollen', 'speichern', 'sprache', 'suchen', 'über', 'und', 'vom', 'von',
  'vor', 'wenn', 'werden', 'wieder', 'wird', 'wirklich', 'wurde', 'wurden',
  'zum', 'zur', 'zurück', 'zwischen',
];
const GERMAN_WORD_RE = new RegExp(
  `(?:^|[^\\p{L}])(?:${GERMAN_WORDS.join('|')})(?:[^\\p{L}]|$)`,
  'iu',
);
const UMLAUT_RE = /[äöüÄÖÜß]/;

export function looksGerman(text) {
  const value = text.trim();
  if (value.length < 3) return false;
  if (UMLAUT_RE.test(value)) return true;
  return GERMAN_WORD_RE.test(value);
}

/** Text without words — icons, separators, punctuation, class strings. */
function isMeaningless(text) {
  const value = text.trim();
  if (!value) return true;
  if (!/\p{L}/u.test(value)) return true;
  return value.replace(/[^\p{L}]/gu, '').length < 2;
}

function loadAllowlist() {
  if (!existsSync(allowlistPath)) return { strings: new Set(), files: [] };
  const parsed = JSON.parse(readFileSync(allowlistPath, 'utf8'));
  return {
    strings: new Set((parsed.strings ?? []).map((entry) => entry.trim())),
    files: parsed.files ?? [],
  };
}

function lineOf(source, offset) {
  return source.slice(0, offset).split('\n').length;
}

function walkTemplate(node, visit) {
  visit(node);
  for (const child of node.children ?? []) {
    if (typeof child === 'object' && child !== null) walkTemplate(child, visit);
  }
  for (const branch of node.branches ?? []) walkTemplate(branch, visit);
}

function scanTemplate(source, descriptor, findings, file) {
  const template = descriptor.template;
  if (!template?.ast) return;

  walkTemplate(template.ast, (node) => {
    if (node.type === 1 && (node.tag === 'style' || node.tag === 'script')) return;

    if (node.type === 2) {
      if (isMeaningless(node.content)) return;
      findings.push({
        file,
        line: lineOf(source, node.loc.start.offset),
        kind: 'text',
        text: node.content.replace(/\s+/g, ' ').trim(),
      });
      return;
    }

    if (node.type === 5) {
      scanTemplateExpression(node.content?.loc?.source ?? '', node.loc.start.offset, source, findings, file);
      return;
    }

    if (node.type !== 1) return;
    for (const prop of node.props ?? []) {
      if (prop.type === 6) {
        if (!TEXT_ATTRS.has(prop.name)) continue;
        const value = prop.value?.content;
        if (!value || isMeaningless(value)) continue;
        findings.push({
          file,
          line: lineOf(source, prop.loc.start.offset),
          kind: `attr:${prop.name}`,
          text: value.replace(/\s+/g, ' ').trim(),
        });
        continue;
      }
      if (
        prop.type === 7
        && prop.name === 'bind'
        && prop.arg?.type === 4
        && prop.arg.isStatic
        && TEXT_ATTRS.has(prop.arg.content)
        && prop.exp?.type === 4
      ) {
        scanTemplateExpression(prop.exp.content, prop.loc.start.offset, source, findings, file);
      }
    }
  });
}

const STRING_RE = /(['"`])((?:\\.|(?!\1)[^\\\n])*)\1/g;

function scanTemplateExpression(expression, offset, source, findings, file) {
  for (const match of expression.matchAll(STRING_RE)) {
    const before = expression.slice(0, match.index).trimEnd();
    if (/(?:\$t|\btc?)\(\s*$/.test(before)) continue;
    if (isMeaningless(match[2])) continue;
    findings.push({
      file,
      line: lineOf(source, offset),
      kind: 'binding',
      text: match[2].replace(/\s+/g, ' ').trim(),
    });
  }
}

/** `new Error(` or `new SomethingError(` immediately before the literal. */
const ERROR_CONSTRUCTION_RE = /\bnew\s+\w*Error\s*\($/;

/** `backend.errors.desktopUnavailable` — a key, not a sentence. */
const MESSAGE_KEY_RE = /^[a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9]+)+$/;

/**
 * Findings for one piece of script source. Exported so the rules can be tested
 * against fixtures instead of against whatever the tree happens to contain.
 */
export function scriptFindings(source, file = 'src/fixture.ts') {
  const findings = [];
  scanScript(source, findings, file);
  return findings;
}

function scanScript(source, findings, file, offsetLine = 0) {
  let inBlockComment = false;

  source.split('\n').forEach((raw, index) => {
    let line = raw;
    if (inBlockComment) {
      const end = line.indexOf('*/');
      if (end === -1) return;
      line = line.slice(end + 2);
      inBlockComment = false;
    }
    const blockStart = line.indexOf('/*');
    if (blockStart !== -1 && !line.slice(blockStart).includes('*/')) {
      line = line.slice(0, blockStart);
      inBlockComment = true;
    }
    const comment = line.indexOf('//');
    if (comment !== -1 && !line.slice(0, comment).includes("'") && !line.slice(0, comment).includes('"')) {
      line = line.slice(0, comment);
    }
    if (/^\s*(?:import|export)\s.*\bfrom\b/.test(line)) return;

    for (const match of line.matchAll(STRING_RE)) {
      if (isMeaningless(match[2])) continue;
      /*
       * A thrown sentence is copy, whatever language it is in. Around 75 call
       * sites interpolate `error.message` into a translated string, so an
       * English literal here reaches the user half-translated — which is how
       * "Verschieben fehlgeschlagen: Desktop backend unavailable." happened.
       * These findings bypass the German heuristic below.
       */
      const thrown = ERROR_CONSTRUCTION_RE.test(line.slice(0, match.index));
      // The fix for a thrown sentence is a key, so a key is not a finding.
      if (thrown && MESSAGE_KEY_RE.test(match[2].trim())) continue;
      findings.push({
        file,
        line: offsetLine + index + 1,
        kind: thrown ? 'error' : 'script',
        text: match[2],
      });
    }
  });
}

/**
 * Rust and Kotlin test modules carry mail fixtures — subjects, snippets, sender
 * names — and those are German on purpose. They are not product copy, and the
 * file-level allowlist cannot tell them apart from the strings around them.
 * Cutting them out keeps the check pointed at what it is for.
 */
function withoutTestModules(source) {
  const start = source.search(/^#\[cfg\(test\)\]/m);
  return start === -1 ? source : source.slice(0, start);
}

export function scanFile(file, source) {
  const findings = [];
  if (/\.(rs|kt)$/.test(file)) {
    scanScript(withoutTestModules(source), findings, file);
    return findings;
  }
  if (file.endsWith('.vue')) {
    const { descriptor } = parse(source, { filename: file });
    scanTemplate(source, descriptor, findings, file);
    for (const block of [descriptor.script, descriptor.scriptSetup]) {
      if (!block) continue;
      scanScript(block.content, findings, file, lineOf(source, block.loc.start.offset) - 1);
    }
  } else {
    scanScript(source, findings, file);
  }
  return findings;
}

function main() {
  const args = process.argv.slice(2);
  const checkMode = args.includes('--check');
  const germanOnly = checkMode || args.includes('--german');
  const jsonOut = args.includes('--json');

  const allowlist = loadAllowlist();
  // Include untracked source files too. A new component must not bypass the
  // check merely because it has not been staged yet.
  const files = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', 'src', 'src-tauri'],
    { cwd: root, encoding: 'utf8' },
  )
    .split('\n')
    .filter((file) => (
      file.endsWith('.vue')
      || file.endsWith('.ts')
      || NATIVE_PRESENTATION_FILES.has(file)
    ))
    .filter((file) => !file.endsWith('.d.ts'))
    .filter((file) => !file.endsWith('.test.ts'))
    .filter((file) => !file.startsWith('src/i18n/'))
    .filter((file) => !allowlist.files.some((prefix) => file === prefix || file.startsWith(prefix)));

  let findings = [];
  for (const file of files) {
    findings.push(...scanFile(file, readFileSync(path.join(root, file), 'utf8')));
  }

  /*
   * Two rules, because one heuristic cannot carry both jobs. In a template
   * every bare string is a finding: after the migration all copy goes through
   * `t()`, so "Papierkorb" — no umlaut, no wordlist hit — would otherwise slip
   * past. In script code the same rule would drown in ids, CSS classes and
   * backend keys, so there the German heuristic decides.
   */
  if (germanOnly) {
    findings = findings.filter(
      (entry) => !['script', 'binding'].includes(entry.kind) || looksGerman(entry.text),
    );
  }
  findings = findings.filter((entry) => !allowlist.strings.has(entry.text.trim()));

  if (jsonOut) {
    process.stdout.write(`${JSON.stringify(findings, null, 2)}\n`);
    return;
  }

  const byFile = new Map();
  for (const entry of findings) {
    if (!byFile.has(entry.file)) byFile.set(entry.file, []);
    byFile.get(entry.file).push(entry);
  }

  for (const [file, entries] of [...byFile].sort((a, b) => b[1].length - a[1].length)) {
    process.stdout.write(`\n${file} (${entries.length})\n`);
    for (const entry of entries) {
      process.stdout.write(`  ${entry.line}\t${entry.kind}\t${entry.text}\n`);
    }
  }

  const total = findings.length;
  if (checkMode) {
    if (total === 0) {
      process.stdout.write('i18n:check — kein deutscher Klartext gefunden.\n');
      return;
    }
    process.stderr.write(
      `\ni18n:check — ${total} Klartext-Strings in ${byFile.size} Dateien.\n`
        + 'Über t() führen oder in scripts/i18n-allowlist.json eintragen.\n',
    );
    process.exitCode = 1;
    return;
  }
  process.stdout.write(`\n${total} findings in ${byFile.size} files\n`);
}

// Only when run as a command: the tests import the rules from here.
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main();
