import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

/*
 * Mock data may enter the app through exactly one door: the command transport.
 *
 * The stores used to carry 13 `mockDataEnabled()` branches that skipped the
 * very backend call under test. A branch that runs differently under test than
 * in production makes a green suite worthless — the From-header bug of 21092a3
 * is the shape of failure that survives one. So: no build-mode flag anywhere,
 * and no fixture import outside `services/backend/mock.ts`, which `vite.config`
 * swaps in for `services/backend/transport`.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');
const MOCK_BACKEND = join(SOURCE_ROOT, 'services/backend/mock.ts');
const REAL_TRANSPORT = join(SOURCE_ROOT, 'services/backend/transport.ts');

/** Build-mode predicates. None of these may exist above the transport. */
const FORBIDDEN_IDENTIFIERS = ['mockDataEnabled', 'mockDataBundled'];
const FIXTURE_MODULE = '@/data/mock';

function sourceFiles(): string[] {
  const files: string[] = [];
  const walk = (dir: string) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (['.ts', '.vue'].includes(extname(entry.name))) {
        files.push(full);
      }
    }
  };
  walk(SOURCE_ROOT);
  return files;
}

function scriptOf(path: string, source: string): string {
  if (extname(path) !== '.vue') return source;
  const blocks = source.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g);
  return [...blocks].map((block) => block[1] ?? '').join('\n');
}

/** Identifier references, so a word inside a comment or a string does not count. */
function referencedIdentifiers(path: string, source: string): Set<string> {
  if (!FORBIDDEN_IDENTIFIERS.some((name) => source.includes(name))) return new Set();
  const file = ts.createSourceFile(path, scriptOf(path, source), ts.ScriptTarget.Latest, true);
  const found = new Set<string>();
  const visit = (node: ts.Node) => {
    if (ts.isIdentifier(node) && FORBIDDEN_IDENTIFIERS.includes(node.text)) found.add(node.text);
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

function importsFixtures(path: string, source: string): boolean {
  if (!source.includes(FIXTURE_MODULE)) return false;
  const file = ts.createSourceFile(path, scriptOf(path, source), ts.ScriptTarget.Latest, true);
  let found = false;
  const visit = (node: ts.Node) => {
    if (
      ts.isImportDeclaration(node)
      && ts.isStringLiteral(node.moduleSpecifier)
      && node.moduleSpecifier.text === FIXTURE_MODULE
    ) {
      found = true;
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

/** The names a module exports, so both transports can be compared. */
function exportedNames(path: string): string[] {
  const file = ts.createSourceFile(path, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true);
  const names: string[] = [];
  for (const statement of file.statements) {
    const exported = ts.canHaveModifiers(statement)
      && ts.getModifiers(statement)?.some((m) => m.kind === ts.SyntaxKind.ExportKeyword);
    if (!exported) continue;
    if (ts.isFunctionDeclaration(statement) && statement.name) names.push(statement.name.text);
    if (ts.isVariableStatement(statement)) {
      for (const declaration of statement.declarationList.declarations) {
        if (ts.isIdentifier(declaration.name)) names.push(declaration.name.text);
      }
    }
  }
  return names.sort();
}

/** Only shell facilities may gate on a native webview. */
const NATIVE_FACILITIES = new Map([
  ['src/stores/connection.ts', 'start'],
  ['src/stores/outbox.ts', 'start'],
  ['src/composables/useComposeWindow.ts', 'supportsNative'],
  ['src/composables/useMailEvents.ts', 'useMailEvents'],
]);

function nativeCommandGates(path: string, source: string): string[] {
  if (!source.includes('backendSupportsNativeCommands')) return [];
  const file = ts.createSourceFile(path, scriptOf(path, source), ts.ScriptTarget.Latest, true);
  const aliases = new Set(['backendSupportsNativeCommands']);
  for (const statement of file.statements) {
    if (!ts.isImportDeclaration(statement)) continue;
    const bindings = statement.importClause?.namedBindings;
    if (bindings && ts.isNamedImports(bindings)) {
      for (const item of bindings.elements) {
        if ((item.propertyName ?? item.name).text === 'backendSupportsNativeCommands') aliases.add(item.name.text);
      }
    }
  }
  const violations: string[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && ((ts.isIdentifier(node.expression) && aliases.has(node.expression.text))
      || (ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'backendSupportsNativeCommands'))) {
      let parent: ts.Node | undefined = node.parent;
      while (parent && !ts.isFunctionDeclaration(parent)) parent = parent.parent;
      const functionName = parent && ts.isFunctionDeclaration(parent) ? parent.name?.text : undefined;
      if (!NATIVE_FACILITIES.has(path) || NATIVE_FACILITIES.get(path) !== functionName) {
        violations.push(path);
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return violations;
}

describe('mock boundary', () => {
  it('has no build-mode branch left in the application', () => {
    const offenders = sourceFiles()
      .filter((path) => referencedIdentifiers(path, readFileSync(path, 'utf8')).size > 0)
      .map((path) => relative(REPOSITORY_ROOT, path));

    expect(offenders).toEqual([]);
  });

  it('uses command availability for stores and command composables', () => {
    const offenders = sourceFiles()
      .filter((path) => !path.endsWith('.test.ts'))
      .filter((path) => /src\/(stores|composables)\//.test(path))
      .flatMap((path) => nativeCommandGates(relative(REPOSITORY_ROOT, path), readFileSync(path, 'utf8')));
    expect(offenders).toEqual([]);
  });

  it('detects native command gates including imported aliases', () => {
    expect(nativeCommandGates('src/stores/planted.ts',
      "import { backendSupportsNativeCommands as native } from '@/services/backend'; function load() { if (native()) send(); }",
    )).toEqual(['src/stores/planted.ts']);
    expect(nativeCommandGates('src/stores/planted.ts',
      "import * as backend from '@/services/backend'; function load() { if (backend.backendSupportsNativeCommands()) send(); }",
    )).toEqual(['src/stores/planted.ts']);
    expect(nativeCommandGates('src/stores/outbox.ts',
      'function load() { if (backendSupportsNativeCommands()) send(); }',
    )).toEqual(['src/stores/outbox.ts']);
    expect(nativeCommandGates('src/stores/outbox.ts',
      'function start() { if (backendSupportsNativeCommands()) listen(); }',
    )).toEqual([]);
  });

  it('reports a store that reaches for a build-mode flag', () => {
    const planted = "import { mockDataEnabled } from '@/utils/mockMode';\n"
      + 'export const seeded = mockDataEnabled() ? [1] : [];\n';

    expect([...referencedIdentifiers('planted.ts', planted)]).toEqual(['mockDataEnabled']);
  });

  it('keeps the demo mailbox behind the mock transport', () => {
    const offenders = sourceFiles()
      .filter((path) => path !== MOCK_BACKEND)
      .filter((path) => importsFixtures(path, readFileSync(path, 'utf8')))
      .map((path) => relative(REPOSITORY_ROOT, path));

    expect(offenders).toEqual([]);
  });

  it('reports a store that imports the demo mailbox directly', () => {
    expect(importsFixtures('planted.ts', "import { mockThreads } from '@/data/mock';\n")).toBe(true);
  });

  /*
   * A swap is only honest if both sides answer the same questions. A name the
   * mock forgets resolves to `undefined` in the browser suite and fails as a
   * "not a function" hundreds of frames away from the cause.
   */
  it('lets the mock transport stand in for the real one', () => {
    expect(exportedNames(MOCK_BACKEND)).toEqual(exportedNames(REAL_TRANSPORT));
  });
});
