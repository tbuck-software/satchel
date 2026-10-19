import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { parse } from 'vue/compiler-sfc';
import { describe, expect, it } from 'vitest';

/*
 * `window.confirm`, `window.prompt` and `window.alert` are not guaranteed to
 * exist in WKWebView. Where they are missing or refused they return `false` or
 * `null`, so a confirmation silently reads as "cancel": blocking a sender,
 * deleting a folder or a rule, restoring a backup — nothing happens and nothing
 * says why. They also cannot be translated or styled, and `prompt` shows a
 * passphrase in clear text.
 *
 * The in-app replacements are `useConfirm().ask()` (yes/no, rendered by
 * `ConfirmDialog`) and `usePrompt().ask()` (one line of text, rendered by
 * `PromptDialog`). Both are mounted once in `App.vue`.
 *
 * Shape follows designSystem.test.ts: a named list, a count and a reason per
 * entry, a budget that may only fall, and tests that guard the list itself.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');
const TEST_SUPPORT_ROOT = join(SOURCE_ROOT, 'test');

const NATIVE_DIALOGS = new Set(['confirm', 'prompt', 'alert']);
const GLOBAL_OBJECTS = new Set(['window', 'globalThis', 'self']);

interface KnownViolation {
  file: string;
  count: number;
  reason: string;
}

/*
 * The calls that predate this scan. The list may shrink and must never grow.
 */
const KNOWN_NATIVE_DIALOGS: KnownViolation[] = [
  {
    file: 'src/components/mail/ThreadView.vue',
    count: 1,
    reason: 'blockSender; moved to useConfirm in its own fix.',
  },
];

// Only ever lower this. See KNOWN_NATIVE_DIALOGS.
const MAX_KNOWN_NATIVE_DIALOGS = 1;

interface NativeDialogCall {
  file: string;
  line: number;
  call: string;
}

function productionSources(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (path === TEST_SUPPORT_ROOT) return [];
    if (entry.isDirectory()) return productionSources(path);
    if (!entry.isFile() || !['.ts', '.vue'].includes(extname(entry.name))) return [];
    if (/\.(?:test|spec)\.[^.]+$/.test(entry.name) || entry.name.endsWith('.d.ts')) return [];
    return [path];
  });
}

function scriptsIn(content: string, path: string): { content: string; lineOffset: number }[] {
  if (extname(path) !== '.vue') return [{ content, lineOffset: 0 }];
  const { descriptor } = parse(content, { filename: path });
  return [descriptor.script, descriptor.scriptSetup]
    .filter((block) => block !== null)
    .map((block) => ({ content: block.content, lineOffset: block.loc.start.line - 1 }));
}

/** Names the file declares itself — `function confirm()` is not the global. */
function declaredNames(sourceFile: ts.SourceFile): Set<string> {
  const names = new Set<string>();
  function collect(name: ts.BindingName | undefined) {
    if (!name) return;
    if (ts.isIdentifier(name)) {
      names.add(name.text);
      return;
    }
    for (const element of name.elements) {
      if (!ts.isOmittedExpression(element)) collect(element.name);
    }
  }
  function visit(node: ts.Node) {
    if (
      ts.isFunctionDeclaration(node)
      || ts.isVariableDeclaration(node)
      || ts.isParameter(node)
      || ts.isImportSpecifier(node)
      || ts.isImportClause(node)
      || ts.isNamespaceImport(node)
    ) {
      collect(node.name as ts.BindingName | undefined);
    }
    ts.forEachChild(node, visit);
  }
  visit(sourceFile);
  return names;
}

function nativeDialogCallsInSource(content: string, path: string): NativeDialogCall[] {
  if (![...NATIVE_DIALOGS].some((name) => content.includes(`${name}(`))) return [];
  return scriptsIn(content, path).flatMap((script) => {
    const sourceFile = ts.createSourceFile(path, script.content, ts.ScriptTarget.Latest, true);
    const declared = declaredNames(sourceFile);
    const calls: NativeDialogCall[] = [];

    function visit(node: ts.Node) {
      if (ts.isCallExpression(node)) {
        const callee = node.expression;
        let call: string | null = null;
        if (
          ts.isPropertyAccessExpression(callee)
          && ts.isIdentifier(callee.expression)
          && GLOBAL_OBJECTS.has(callee.expression.text)
          && !declared.has(callee.expression.text)
          && NATIVE_DIALOGS.has(callee.name.text)
        ) {
          call = `${callee.expression.text}.${callee.name.text}`;
        } else if (
          ts.isIdentifier(callee)
          && NATIVE_DIALOGS.has(callee.text)
          && !declared.has(callee.text)
        ) {
          call = callee.text;
        }
        if (call) {
          const line = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line;
          calls.push({ file: path, line: line + script.lineOffset + 1, call });
        }
      }
      ts.forEachChild(node, visit);
    }

    visit(sourceFile);
    return calls;
  });
}

function nativeDialogCalls(): NativeDialogCall[] {
  return productionSources(SOURCE_ROOT)
    .flatMap((path) => nativeDialogCallsInSource(readFileSync(path, 'utf8'), path))
    .map((entry) => ({ ...entry, file: relative(REPOSITORY_ROOT, entry.file) }));
}

function countByFile(calls: NativeDialogCall[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const call of calls) counts[call.file] = (counts[call.file] ?? 0) + 1;
  return counts;
}

describe('native browser dialogs', () => {
  it('finds window.confirm, bare prompt and globalThis.alert, but not local functions', () => {
    const source = [
      "if (!window.confirm('Delete?')) return;",
      "const value = prompt('Name');",
      "globalThis.alert('Done');",
      'function confirm() { return true; }',
      'confirm();',
      'linkOpener.confirm();',
    ].join('\n');

    expect(nativeDialogCallsInSource(source, '/virtual/example.ts')).toEqual([
      { file: '/virtual/example.ts', line: 1, call: 'window.confirm' },
      { file: '/virtual/example.ts', line: 2, call: 'prompt' },
      { file: '/virtual/example.ts', line: 3, call: 'globalThis.alert' },
    ]);
  });

  it('reads both script blocks of a single-file component', () => {
    const source = [
      '<script setup lang="ts">',
      "const ok = window.confirm('Really?');",
      '</script>',
      '<template><p /></template>',
    ].join('\n');

    expect(nativeDialogCallsInSource(source, '/virtual/Example.vue')).toEqual([
      { file: '/virtual/Example.vue', line: 2, call: 'window.confirm' },
    ]);
  });

  it('allows no native dialog beyond the known list; use useConfirm or usePrompt', () => {
    const counts = countByFile(nativeDialogCalls());
    const known = Object.fromEntries(KNOWN_NATIVE_DIALOGS.map((entry) => [entry.file, entry.count]));
    expect(counts).toEqual(known);
  });

  it('has no stale entries in the known list', () => {
    const counts = countByFile(nativeDialogCalls());
    const stale = KNOWN_NATIVE_DIALOGS.filter((entry) => !counts[entry.file]);
    expect(stale).toEqual([]);
  });

  it('keeps the known list within its budget', () => {
    const total = KNOWN_NATIVE_DIALOGS.reduce((sum, entry) => sum + entry.count, 0);
    expect(total).toBeLessThanOrEqual(MAX_KNOWN_NATIVE_DIALOGS);
  });
});
