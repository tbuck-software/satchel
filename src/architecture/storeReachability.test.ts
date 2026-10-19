import { readdirSync, readFileSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { describe, expect, it } from 'vitest';

/*
 * A store method nobody calls is a second implementation waiting to happen:
 * the tests exercise it, the UI takes another path, and the two drift until a
 * failure in the real one surfaces in production. ARCH-05 found three of them
 * behind `thread.isStarred` alone.
 *
 * This scan lists every method the threads façade returns that no production
 * file outside `src/stores/` mentions. Shape follows the other architecture
 * scans: a named list with a reason per entry and a budget that may only fall.
 * Adding an entry therefore costs two deliberate edits.
 */

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');
const STORE_ROOT = join(SOURCE_ROOT, 'stores');
const TEST_SUPPORT_ROOT = join(SOURCE_ROOT, 'test');
const FACADE = join(STORE_ROOT, 'threads.ts');
const MODULE_ROOT = join(STORE_ROOT, 'threads');

/*
 * Methods without a caller today. Each one is a decision someone still owes:
 * wire it up because the UI needs it, or delete it because nobody wants it.
 */
const KNOWN_UNREACHED: { method: string; reason: string }[] = [];
const MAX_KNOWN_UNREACHED = 0;

interface ReachabilityInput {
  /** The façade whose returned object is the store's public surface. */
  facade: string;
  /** Sources that declare the members, the façade included. */
  modules: string[];
  /** Production sources that may call the store. */
  consumers: string[];
}

/** The name one object-literal member defines, shorthand or `name: value`. */
function memberName(property: ts.ObjectLiteralElementLike): string {
  if (ts.isShorthandPropertyAssignment(property)) return property.name.text;
  if (ts.isPropertyAssignment(property) && ts.isIdentifier(property.name)) {
    return property.name.text;
  }
  throw new Error('Unsupported member in the store surface');
}

function literalNames(literal: ts.ObjectLiteralExpression): string[] {
  return literal.properties.map(memberName);
}

/** The object literal a `const name = {…}` in `scope` holds, if any. */
function constLiteral(scope: ts.Node, name: string): ts.ObjectLiteralExpression | undefined {
  let found: ts.ObjectLiteralExpression | undefined;
  const visit = (node: ts.Node) => {
    if (
      ts.isVariableDeclaration(node)
      && ts.isIdentifier(node.name)
      && node.name.text === name
      && node.initializer
      && ts.isObjectLiteralExpression(node.initializer)
    ) {
      found ??= node.initializer;
    }
    ts.forEachChild(node, visit);
  };
  visit(scope);
  return found;
}

/**
 * The `surface` a module factory returns: `surface: {…}` in its return, or
 * `surface` pointing at a `const surface = {…}` in its body.
 */
function factorySurface(factory: string, modules: string[]): string[] {
  for (const source of modules) {
    const file = ts.createSourceFile('module.ts', source, ts.ScriptTarget.Latest, true);
    const declaration = file.statements.find(
      (statement): statement is ts.FunctionDeclaration =>
        ts.isFunctionDeclaration(statement) && statement.name?.text === factory,
    );
    if (!declaration?.body) continue;
    let names: string[] | undefined;
    const visit = (node: ts.Node) => {
      if (names) return;
      if (ts.isPropertyAssignment(node) && ts.isIdentifier(node.name) && node.name.text === 'surface'
        && ts.isObjectLiteralExpression(node.initializer)) {
        names = literalNames(node.initializer);
        return;
      }
      if (ts.isShorthandPropertyAssignment(node) && node.name.text === 'surface') {
        const literal = constLiteral(declaration, 'surface');
        if (literal) names = literalNames(literal);
        return;
      }
      ts.forEachChild(node, visit);
    };
    visit(declaration.body);
    if (names) return names;
  }
  throw new Error(`No surface found for ${factory}`);
}

/**
 * The names a `defineStore` setup returns — the store's public surface.
 *
 * The threads façade spreads module surfaces rather than listing members, so a
 * spread is followed to where its names are written down: `...own` to the
 * `const own = {…}` beside it, `...part.surface` to the factory that built
 * `part`. Anything else is an error, not a silent gap in the scan.
 */
function storeMembers(facade: string, modules: string[] = [facade]): string[] {
  const file = ts.createSourceFile('facade.ts', facade, ts.ScriptTarget.Latest, true);
  let setup: ts.Node | undefined;
  const findSetup = (node: ts.Node) => {
    if (
      ts.isCallExpression(node)
      && ts.isIdentifier(node.expression)
      && node.expression.text === 'defineStore'
    ) {
      const last = node.arguments.at(-1);
      if (last && (ts.isArrowFunction(last) || ts.isFunctionExpression(last))) setup = last.body;
    }
    ts.forEachChild(node, findSetup);
  };
  findSetup(file);
  if (!setup || !ts.isBlock(setup)) throw new Error('No defineStore setup body found');
  const body = setup;

  const returned = body.statements.find(ts.isReturnStatement)?.expression;
  if (!returned || !ts.isObjectLiteralExpression(returned)) {
    throw new Error('The store setup does not return an object literal');
  }
  return returned.properties.flatMap((property) => {
    if (!ts.isSpreadAssignment(property)) return [memberName(property)];
    const spread = property.expression;
    if (ts.isIdentifier(spread)) {
      const literal = constLiteral(body, spread.text);
      if (!literal) throw new Error(`Cannot resolve spread ...${spread.text}`);
      return literalNames(literal);
    }
    if (
      ts.isPropertyAccessExpression(spread)
      && spread.name.text === 'surface'
      && ts.isIdentifier(spread.expression)
    ) {
      const part = spread.expression.text;
      let factory: string | undefined;
      const findFactory = (node: ts.Node) => {
        if (
          ts.isVariableDeclaration(node)
          && ts.isIdentifier(node.name)
          && node.name.text === part
          && node.initializer
          && ts.isCallExpression(node.initializer)
          && ts.isIdentifier(node.initializer.expression)
        ) {
          factory = node.initializer.expression.text;
        }
        ts.forEachChild(node, findFactory);
      };
      findFactory(body);
      if (!factory) throw new Error(`Cannot resolve spread ...${part}.surface`);
      return factorySurface(factory, modules);
    }
    throw new Error('Unsupported spread in the store surface');
  });
}

/**
 * Refs and computed values are left out: a template reads them by name in
 * markup this scan does not resolve, so absence there proves nothing.
 */
function isMethod(name: string, modules: string[]): boolean {
  const declaration = new RegExp(`(?:^|\\n)\\s*(?:export\\s+)?(?:async\\s+)?function ${name}\\s*\\(`);
  return modules.some((source) => declaration.test(source));
}

function unreachedStoreMethods({ facade, modules, consumers }: ReachabilityInput): string[] {
  return storeMembers(facade, modules)
    .filter((name) => isMethod(name, modules))
    .filter((name) => !consumers.some((source) => new RegExp(`\\b${name}\\b`).test(source)));
}

function sourcesIn(root: string, keep: (path: string) => boolean): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (entry.isDirectory()) return sourcesIn(path, keep);
    if (!entry.isFile() || !['.ts', '.tsx', '.vue'].includes(extname(entry.name))) return [];
    return keep(path) ? [path] : [];
  });
}

function read(paths: string[]): string[] {
  return paths.map((path) => readFileSync(path, 'utf8'));
}

function repositoryInput(): ReachabilityInput {
  const consumers = sourcesIn(SOURCE_ROOT, (path) => (
    !path.startsWith(STORE_ROOT)
    && !path.startsWith(TEST_SUPPORT_ROOT)
    && !/\.(?:test|spec)\.[^.]+$/.test(relative(SOURCE_ROOT, path))
  ));
  return {
    facade: readFileSync(FACADE, 'utf8'),
    modules: read([FACADE, ...sourcesIn(MODULE_ROOT, () => true)]),
    consumers: read(consumers),
  };
}

describe('threads store reachability', () => {
  const unreached = unreachedStoreMethods(repositoryInput());

  it('reports exactly the methods the list names', () => {
    expect([...unreached].sort()).toEqual(KNOWN_UNREACHED.map((entry) => entry.method).sort());
  });

  it('keeps no entry for a method the façade no longer returns', () => {
    const { facade, modules } = repositoryInput();
    const methods = storeMembers(facade, modules).filter((name) => isMethod(name, modules));
    for (const entry of KNOWN_UNREACHED) expect(methods).toContain(entry.method);
  });

  it('stays within the budget', () => {
    expect(KNOWN_UNREACHED.length).toBeLessThanOrEqual(MAX_KNOWN_UNREACHED);
  });
});

describe('the reachability scan itself', () => {
  const facade = `
    export const useThreadsStore = defineStore('threads', () => {
      const threads = ref([]);
      function usedMethod() {}
      function orphanMethod() {}
      return { threads, usedMethod, orphanMethod };
    });
  `;

  it('reports a method no consumer calls', () => {
    expect(unreachedStoreMethods({
      facade,
      modules: [facade],
      consumers: ['store.usedMethod();'],
    })).toEqual(['orphanMethod']);
  });

  it('follows spread module surfaces to the names they define', () => {
    const spreadFacade = `
      export const useThreadsStore = defineStore('threads', () => {
        const selection = createThreadSelection({});
        function discardThread() {}
        const own = { discardThread };
        return { ...selection.surface, ...own };
      });
    `;
    const selectionModule = `
      export function createThreadSelection() {
        function orphanMethod() {}
        function clearSelection() {}
        return { surface: { orphanMethod, clearSelection }, startWatching() {} };
      }
    `;

    expect(unreachedStoreMethods({
      facade: spreadFacade,
      modules: [spreadFacade, selectionModule],
      consumers: ['store.clearSelection(); store.discardThread();'],
    })).toEqual(['orphanMethod']);
  });

  it('refuses a spread it cannot resolve instead of scanning less', () => {
    const opaque = `
      export const useThreadsStore = defineStore('threads', () => {
        return { ...somewhereElse() };
      });
    `;

    expect(() => storeMembers(opaque)).toThrow('Unsupported spread');
  });

  it('does not accept a substring as a caller', () => {
    expect(unreachedStoreMethods({
      facade,
      modules: [facade],
      consumers: ['store.usedMethodLater(); store.orphanMethod();'],
    })).toEqual(['usedMethod']);
  });
});
