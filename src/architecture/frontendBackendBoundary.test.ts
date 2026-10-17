import { readdirSync, readFileSync } from 'node:fs';
import { dirname, extname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { compileScript, compileTemplate, parse } from 'vue/compiler-sfc';
import { describe, expect, it } from 'vitest';

interface ScriptSource {
  content: string;
  lineOffset: number;
  scriptKind: ts.ScriptKind;
}

type BoundaryKind = 'backend' | 'tauri';

interface BoundaryImport {
  file: string;
  line: number;
  specifier: string;
  kind: BoundaryKind;
}

interface LocalStorageAccess {
  file: string;
  line: number;
}

const REPOSITORY_ROOT = fileURLToPath(new URL('../..', import.meta.url));
const SOURCE_ROOT = join(REPOSITORY_ROOT, 'src');
const BACKEND_ROOT = join(SOURCE_ROOT, 'services', 'backend');
const PERSISTENCE_ROOT = join(SOURCE_ROOT, 'persistence');
const TEST_SUPPORT_ROOT = join(SOURCE_ROOT, 'test');
const STORAGE_ADAPTER = join(PERSISTENCE_ROOT, 'index.ts');
const GUARDED_ROOTS = [
  join(SOURCE_ROOT, 'components'),
  join(SOURCE_ROOT, 'views'),
];

/*
 * ADR 0012: settings panes may send their forms straight to a command, a store
 * gains nothing there. The exemption covers `services/backend` only — reaching
 * past it into `@tauri-apps` stays a violation everywhere. Any further root
 * needs an ADR that says the same thing about it.
 */
const ALLOWED_DIRECT_BACKEND_ROOTS = [
  join(SOURCE_ROOT, 'components', 'settings'),
];

/*
 * The violations that predate this scan, counted per file. The list may shrink
 * and must never grow: a new violation makes the scan red, and putting it here
 * also means raising MAX_KNOWN_VIOLATIONS below — two deliberate edits, not an
 * accident. Removing the last violation from a file means removing its entry;
 * a stale entry is a failure too.
 */
const KNOWN_BOUNDARY_VIOLATIONS: { file: string; count: number; reason: string }[] = [
  {
    file: 'src/components/contacts/PersonProfileSheet.vue',
    count: 1,
    reason: 'Blocking a sender belongs behind a contacts store.',
  },
  {
    file: 'src/components/navigation/OmniSearch.vue',
    count: 1,
    reason: 'Contact type only; the type belongs to a store-facing model.',
  },
  {
    file: 'src/components/onboarding/OnboardingFlow.vue',
    count: 1,
    reason: 'Account setup runs the same commands as the settings panes.',
  },
  {
    file: 'src/components/plugins/PluginSlotHost.vue',
    count: 1,
    reason: 'Plugin slots have no store yet.',
  },
];

// Only ever lower this. See KNOWN_BOUNDARY_VIOLATIONS.
const MAX_KNOWN_VIOLATIONS = 4;

// A file can only reach past the boundary through a specifier spelling one of
// these, so they double as the substring gate in front of the parser.
const BOUNDARY_NEEDLES = ['services/backend', '@tauri-apps'];

function productionSources(root: string): string[] {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    if (path === TEST_SUPPORT_ROOT) return [];
    if (entry.isDirectory()) return productionSources(path);
    if (!entry.isFile() || !['.ts', '.tsx', '.vue'].includes(extname(entry.name))) return [];
    if (/\.(?:test|spec)\.[^.]+$/.test(entry.name)) return [];
    return [path];
  });
}

function scriptsInSource(content: string, path: string): ScriptSource[] {
  if (extname(path) !== '.vue') {
    return [{
      content,
      lineOffset: 0,
      scriptKind: extname(path) === '.tsx' ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    }];
  }

  const { descriptor, errors } = parse(content, { filename: path });
  if (errors.length > 0) {
    throw new Error(`Could not parse ${relative(REPOSITORY_ROOT, path)}: ${errors.join(', ')}`);
  }

  return [descriptor.script, descriptor.scriptSetup]
    .filter((block) => block !== null)
    .map((block) => ({
      content: block.content,
      lineOffset: block.loc.start.line - 1,
      scriptKind: block.lang === 'tsx' ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
    }));
}

function scriptsIn(path: string): ScriptSource[] {
  return scriptsInSource(readFileSync(path, 'utf8'), path);
}

function classifySpecifier(specifier: string, containingFile: string): BoundaryKind | null {
  if (/^@tauri-apps\/api(?:$|\/)/.test(specifier)) return 'tauri';
  if (specifier.startsWith('@tauri-apps/plugin-')) return 'tauri';
  if (/^@\/services\/backend(?:$|\/)/.test(specifier)) return 'backend';
  if (!specifier.startsWith('.')) return null;

  const resolved = resolve(dirname(containingFile), specifier);
  const isBackend = resolved === BACKEND_ROOT || resolved.startsWith(`${BACKEND_ROOT}/`);
  return isBackend ? 'backend' : null;
}

function isUnder(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

function literalText(node: ts.Node | undefined): string | null {
  return node && ts.isStringLiteralLike(node) ? node.text : null;
}

function boundaryImportsIn(script: ScriptSource, path: string): BoundaryImport[] {
  const sourceFile = ts.createSourceFile(
    path,
    script.content,
    ts.ScriptTarget.Latest,
    true,
    script.scriptKind,
  );
  const imports: BoundaryImport[] = [];

  function record(specifier: string | null, node: ts.Node) {
    const kind = specifier ? classifySpecifier(specifier, path) : null;
    if (!specifier || !kind) return;
    const line = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line;
    imports.push({ file: path, line: line + script.lineOffset + 1, specifier, kind });
  }

  function visit(node: ts.Node) {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) {
      record(literalText(node.moduleSpecifier), node);
    } else if (
      ts.isImportEqualsDeclaration(node)
      && ts.isExternalModuleReference(node.moduleReference)
    ) {
      record(literalText(node.moduleReference.expression), node);
    } else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) {
      record(literalText(node.argument.literal), node);
    } else if (ts.isCallExpression(node)) {
      const isDynamicImport = node.expression.kind === ts.SyntaxKind.ImportKeyword;
      const isRequire = ts.isIdentifier(node.expression) && node.expression.text === 'require';
      if (isDynamicImport || isRequire) record(literalText(node.arguments[0]), node);
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return imports;
}

/*
 * Parsing every production file takes seconds and blew the 10 s budget on a
 * loaded CI runner. A file can only reach past the boundary through a
 * specifier that spells one of BOUNDARY_NEEDLES, and it can only touch
 * `localStorage` through a token that spells it, so a substring gate skips
 * the parser for the rest without changing what the scans can find.
 */
function mayContain(path: string, needle: string): boolean {
  return readFileSync(path, 'utf8').includes(needle);
}

function boundaryImportsInSource(content: string, path: string): BoundaryImport[] {
  if (!BOUNDARY_NEEDLES.some((needle) => content.includes(needle))) return [];
  return scriptsInSource(content, path).flatMap((script) => boundaryImportsIn(script, path));
}

function boundaryViolationsInSource(content: string, path: string): BoundaryImport[] {
  if (!GUARDED_ROOTS.some((root) => isUnder(path, root))) return [];
  const allowsDirectBackend = ALLOWED_DIRECT_BACKEND_ROOTS.some((root) => isUnder(path, root));
  return boundaryImportsInSource(content, path)
    .filter((entry) => !(entry.kind === 'backend' && allowsDirectBackend));
}

function boundaryViolations(): BoundaryImport[] {
  return GUARDED_ROOTS
    .flatMap(productionSources)
    .flatMap((path) => boundaryViolationsInSource(readFileSync(path, 'utf8'), path))
    .map((entry) => ({ ...entry, file: relative(REPOSITORY_ROOT, entry.file) }));
}

function localStorageAccessesIn(script: ScriptSource, path: string): LocalStorageAccess[] {
  const compilerOptions: ts.CompilerOptions = {
    noLib: true,
    target: ts.ScriptTarget.Latest,
  };
  const compilerPath = extname(path) === '.vue' ? `${path}.ts` : path;
  const parsedSource = ts.createSourceFile(
    compilerPath,
    script.content,
    ts.ScriptTarget.Latest,
    true,
    script.scriptKind,
  );
  const host = ts.createCompilerHost(compilerOptions, true);
  host.fileExists = (fileName) => fileName === compilerPath;
  host.readFile = (fileName) => (fileName === compilerPath ? script.content : undefined);
  host.getSourceFile = (fileName) => (fileName === compilerPath ? parsedSource : undefined);
  const program = ts.createProgram({ rootNames: [compilerPath], options: compilerOptions, host });
  const sourceFile = program.getSourceFile(compilerPath);
  if (!sourceFile) throw new Error(`Could not bind ${relative(REPOSITORY_ROOT, path)}`);
  const boundSourceFile = sourceFile;
  const checker = program.getTypeChecker();
  const accesses: LocalStorageAccess[] = [];
  const recordedPositions = new Set<number>();

  function record(node: ts.Node) {
    const position = node.getStart(boundSourceFile);
    if (recordedPositions.has(position)) return;
    recordedPositions.add(position);
    const line = boundSourceFile.getLineAndCharacterOfPosition(node.getStart(boundSourceFile)).line;
    accesses.push({ file: path, line: line + script.lineOffset + 1 });
  }

  function isUnshadowedGlobal(node: ts.Expression): boolean {
    if (
      ts.isParenthesizedExpression(node)
      || ts.isAsExpression(node)
      || ts.isTypeAssertionExpression(node)
      || ts.isNonNullExpression(node)
      || ts.isSatisfiesExpression(node)
    ) {
      return isUnshadowedGlobal(node.expression);
    }
    if (ts.isIdentifier(node)) {
      const symbol = checker.getSymbolAtLocation(node);
      const isLocallyDeclared = symbol?.declarations?.some(
        (declaration) => declaration.getSourceFile() === boundSourceFile,
      ) ?? false;
      return ['window', 'globalThis', 'self'].includes(node.text)
        && !isLocallyDeclared;
    }
    if (ts.isPropertyAccessExpression(node)) {
      if (
        ts.isIdentifier(node.expression)
        && node.expression.text === 'document'
        && node.name.text === 'defaultView'
        && !(checker.getSymbolAtLocation(node.expression)?.declarations?.some(
          (declaration) => declaration.getSourceFile() === boundSourceFile,
        ) ?? false)
      ) {
        return true;
      }
      return isUnshadowedGlobal(node.expression);
    }
    return false;
  }

  function isGlobalDestructuringBinding(node: ts.Identifier): boolean {
    if (!ts.isBindingElement(node.parent)) return false;
    const binding = node.parent;
    if (binding.name !== node) return false;
    const property = binding.propertyName ?? binding.name;
    const propertyName = ts.isComputedPropertyName(property)
      ? literalText(property.expression)
      : ts.isIdentifier(property) || ts.isStringLiteralLike(property)
        ? property.text
        : null;
    if (propertyName !== 'localStorage') return false;
    if (!ts.isObjectBindingPattern(binding.parent)) return false;
    const declaration = binding.parent.parent;
    return ts.isVariableDeclaration(declaration)
      && declaration.initializer !== undefined
      && isUnshadowedGlobal(declaration.initializer);
  }

  function isGlobalDestructuringAssignment(node: ts.Node): boolean {
    if (!ts.isPropertyAssignment(node.parent)) return false;
    const property = node.parent;
    if (property.initializer !== node) return false;
    const propertyName = ts.isComputedPropertyName(property.name)
      ? literalText(property.name.expression)
      : ts.isIdentifier(property.name) || ts.isStringLiteralLike(property.name)
        ? property.name.text
        : null;
    if (propertyName !== 'localStorage' || !ts.isObjectLiteralExpression(property.parent)) {
      return false;
    }

    let assignmentTarget: ts.Expression = property.parent;
    while (ts.isParenthesizedExpression(assignmentTarget.parent)) {
      assignmentTarget = assignmentTarget.parent;
    }
    const assignment = assignmentTarget.parent;
    return ts.isBinaryExpression(assignment)
      && assignment.operatorToken.kind === ts.SyntaxKind.EqualsToken
      && assignment.left === assignmentTarget
      && isUnshadowedGlobal(assignment.right);
  }

  function visit(node: ts.Node) {
    if (
      ts.isPropertyAccessExpression(node)
      && node.name.text === 'localStorage'
      && isUnshadowedGlobal(node.expression)
    ) {
      record(node.name);
    } else if (
      ts.isElementAccessExpression(node)
      && literalText(node.argumentExpression) === 'localStorage'
      && isUnshadowedGlobal(node.expression)
    ) {
      record(node.argumentExpression);
    } else if (ts.isIdentifier(node)) {
      if (isGlobalDestructuringBinding(node) || isGlobalDestructuringAssignment(node)) {
        record(node);
      } else if (node.text === 'localStorage') {
        const isPropertyName = (
          ts.isPropertyAccessExpression(node.parent)
          && node.parent.name === node
        ) || (
          ts.isBindingElement(node.parent)
          && node.parent.propertyName === node
        ) || (
          ts.isPropertyAssignment(node.parent)
          && node.parent.name === node
        );
        const isLocallyDeclared = checker.getSymbolAtLocation(node)?.declarations?.some(
          (declaration) => declaration.getSourceFile() === boundSourceFile,
        ) ?? false;
        if (!isPropertyName && !isLocallyDeclared) record(node);
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(boundSourceFile);
  return accesses;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function templateExpressionAccessLines(root: unknown): number[] {
  const lines = new Set<number>();
  const visited = new WeakSet<object>();

  function expressionLine(node: Record<string, unknown>): number {
    const loc = isRecord(node.loc) && isRecord(node.loc.start) ? node.loc.start.line : undefined;
    return typeof loc === 'number' ? loc : 1;
  }

  function unwrappedExpression(node: unknown): unknown {
    let current = node;
    while (
      isRecord(current)
      && [
        'ChainExpression',
        'ParenthesizedExpression',
        'TSAsExpression',
        'TSNonNullExpression',
        'TSTypeAssertion',
      ].includes(String(current.type))
    ) {
      current = current.expression;
    }
    return current;
  }

  function isMemberExpression(node: Record<string, unknown>): boolean {
    return node.type === 'MemberExpression' || node.type === 'OptionalMemberExpression';
  }

  function propertyIs(node: unknown, name: string): boolean {
    if (!isRecord(node)) return false;
    if (
      (node.type === 'Identifier' && node.name === name)
      || (node.type === 'StringLiteral' && node.value === name)
    ) {
      return true;
    }
    if (node.type !== 'TemplateLiteral' || !Array.isArray(node.expressions)) return false;
    if (node.expressions.length > 0 || !Array.isArray(node.quasis) || node.quasis.length !== 1) {
      return false;
    }
    const quasi = node.quasis[0];
    return isRecord(quasi)
      && isRecord(quasi.value)
      && (quasi.value.cooked === name || quasi.value.raw === name);
  }

  function isTemplateGlobal(node: unknown): boolean {
    const expression = unwrappedExpression(node);
    if (!isRecord(expression)) return false;
    if (expression.type === 'Identifier') {
      return ['_ctx.window', '_ctx.globalThis', '_ctx.self'].includes(String(expression.name));
    }
    if (!isMemberExpression(expression)) return false;
    if (
      isRecord(expression.object)
      && expression.object.type === 'Identifier'
      && expression.object.name === '_ctx.document'
      && propertyIs(expression.property, 'defaultView')
    ) {
      return true;
    }
    return isTemplateGlobal(expression.object);
  }

  function propertyIsLocalStorage(node: unknown): boolean {
    return propertyIs(node, 'localStorage');
  }

  function visitExpression(node: unknown) {
    if (!isRecord(node) || visited.has(node)) return;
    visited.add(node);

    if (node.type === 'Identifier' && node.name === '_ctx.localStorage') {
      lines.add(expressionLine(node));
    } else if (
      (node.type === 'MemberExpression' || node.type === 'OptionalMemberExpression')
      && isTemplateGlobal(node.object)
      && propertyIsLocalStorage(node.property)
    ) {
      lines.add(expressionLine(isRecord(node.property) ? node.property : node));
    }

    for (const [key, value] of Object.entries(node)) {
      if (key === 'loc' || key === 'extra') continue;
      if (Array.isArray(value)) value.forEach(visitExpression);
      else visitExpression(value);
    }
  }

  visitExpression(root);
  return Array.from(lines);
}

function templateLocalStorageAccessesInSource(
  source: string,
  path: string,
  lineOffset: number,
  bindingMetadata?: NonNullable<
    NonNullable<Parameters<typeof compileTemplate>[0]['compilerOptions']>['bindingMetadata']
  >,
): LocalStorageAccess[] {
  const compiled = compileTemplate({
    source,
    filename: path,
    id: path,
    compilerOptions: { bindingMetadata, expressionPlugins: ['typescript'] },
  });
  if (compiled.errors.length > 0) {
    throw new Error(
      `Could not compile ${relative(REPOSITORY_ROOT, path)} template: ${compiled.errors.join(', ')}`,
    );
  }

  const accessLines = new Set<number>();
  const visited = new WeakSet<object>();

  function visitTemplate(node: unknown) {
    if (!isRecord(node) || visited.has(node)) return;
    visited.add(node);

    if (isRecord(node.ast) && isRecord(node.loc) && isRecord(node.loc.start)) {
      const expressionStartLine = node.loc.start.line;
      if (typeof expressionStartLine === 'number') {
        for (const relativeLine of templateExpressionAccessLines(node.ast)) {
          accessLines.add(lineOffset + expressionStartLine + relativeLine - 1);
        }
      }
    } else if (
      node.type === 4
      && typeof node.content === 'string'
      && /(?:^|[^\w$])_ctx\.localStorage(?:$|[^\w$])/.test(node.content)
      && isRecord(node.loc)
      && isRecord(node.loc.start)
      && typeof node.loc.start.line === 'number'
    ) {
      accessLines.add(lineOffset + node.loc.start.line);
    }

    for (const [key, value] of Object.entries(node)) {
      if (key === 'ast' || key === 'loc' || key === 'codegenNode') continue;
      if (Array.isArray(value)) value.forEach(visitTemplate);
      else visitTemplate(value);
    }
  }

  visitTemplate(compiled.ast);
  return Array.from(accessLines, (line) => ({ file: path, line }));
}

function templateLocalStorageAccessesInSfcSource(
  content: string,
  path: string,
): LocalStorageAccess[] {
  const { descriptor, errors } = parse(content, { filename: path });
  if (errors.length > 0) {
    throw new Error(`Could not parse ${relative(REPOSITORY_ROOT, path)}: ${errors.join(', ')}`);
  }
  if (!descriptor.template) return [];
  const bindingMetadata = descriptor.scriptSetup
    ? compileScript(descriptor, { id: path }).bindings
    : undefined;
  return templateLocalStorageAccessesInSource(
    descriptor.template.content,
    path,
    descriptor.template.loc.start.line - 1,
    bindingMetadata,
  );
}

function templateLocalStorageAccessesInFile(path: string): LocalStorageAccess[] {
  if (extname(path) !== '.vue') return [];
  return templateLocalStorageAccessesInSfcSource(readFileSync(path, 'utf8'), path);
}

function localStorageAccessesInFile(path: string): LocalStorageAccess[] {
  if (!mayContain(path, 'localStorage')) return [];
  return [
    ...scriptsIn(path).flatMap((script) => localStorageAccessesIn(script, path)),
    ...templateLocalStorageAccessesInFile(path),
  ];
}

function violationCountsByFile(violations: BoundaryImport[]): Map<string, number> {
  const counts = new Map<string, number>();
  for (const entry of violations) {
    counts.set(entry.file, (counts.get(entry.file) ?? 0) + 1);
  }
  return counts;
}

describe('frontend backend boundary', () => {
  it('keeps every component and view behind stores, composables, and services', () => {
    const violations = boundaryViolations();
    const counts = violationCountsByFile(violations);
    const known = new Map(KNOWN_BOUNDARY_VIOLATIONS.map((entry) => [entry.file, entry.count]));
    const unexpected = violations
      .filter((entry) => (counts.get(entry.file) ?? 0) !== known.get(entry.file))
      .map((entry) => `${entry.file}:${entry.line} imports ${entry.specifier}`);
    const details = unexpected.join('\n');

    expect(Object.fromEntries(counts), details || undefined)
      .toEqual(Object.fromEntries(known));
  });

  it('carries no stale entry on the known violation list', () => {
    const counts = violationCountsByFile(boundaryViolations());
    const stale = KNOWN_BOUNDARY_VIOLATIONS
      .filter((entry) => (counts.get(entry.file) ?? 0) < entry.count)
      .map((entry) => {
        return `${entry.file} is listed ${entry.count}x, found ${counts.get(entry.file) ?? 0}`;
      });

    expect(stale, 'the list may shrink — drop what is fixed').toEqual([]);
  });

  it('holds the known violation list to its budget', () => {
    // A new violation needs an entry here *and* a raised budget. Both by hand.
    const total = KNOWN_BOUNDARY_VIOLATIONS.reduce((sum, entry) => sum + entry.count, 0);

    expect(total).toBeLessThanOrEqual(MAX_KNOWN_VIOLATIONS);
  });

  it('recognizes every supported module syntax without matching plain text', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'mail', 'Example.ts');
    const source = `
      import { invoke } from '@/services/backend';
      import type { BackendThread } from '@/services/backend/threads';
      export { backendSendMessage } from '../../services/backend';
      const dynamic = import('@/services/backend/composer');
      const commonJs = require('@/services/backend/messages');
      import backend = require('@/services/backend');
      type InvokeResult = import('@/services/backend').InvokeResult;
      import { getCurrentWindow } from '@tauri-apps/api/window';
      const dialog = import('@tauri-apps/plugin-dialog');
      import * as api from '@tauri-apps/api';
      const harmless = '@/services/backend';
      const alsoHarmless = '@tauri-apps/api/core';
      // import { ignored } from '@/services/backend';
      import type { Plugin } from '@tauri-apps-community/unrelated';
    `;
    const imports = boundaryImportsIn({
      content: source,
      lineOffset: 0,
      scriptKind: ts.ScriptKind.TS,
    }, syntheticPath);

    expect(imports.map((entry) => `${entry.kind} ${entry.specifier}`)).toEqual([
      'backend @/services/backend',
      'backend @/services/backend/threads',
      'backend ../../services/backend',
      'backend @/services/backend/composer',
      'backend @/services/backend/messages',
      'backend @/services/backend',
      'backend @/services/backend',
      'tauri @tauri-apps/api/window',
      'tauri @tauri-apps/plugin-dialog',
      'tauri @tauri-apps/api',
    ]);
  });

  it('rejects a direct @tauri-apps/api import in a guarded component', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'topics', 'TopicBoard.vue');
    const source = [
      '<script setup lang="ts">',
      "import { ref } from 'vue';",
      "import { invoke } from '@tauri-apps/api/core';",
      'const topics = ref([]);',
      '</script>',
      '<template><div /></template>',
    ].join('\n');

    expect(boundaryViolationsInSource(source, syntheticPath)).toEqual([
      {
        file: syntheticPath,
        line: 3,
        specifier: '@tauri-apps/api/core',
        kind: 'tauri',
      },
    ]);
  });

  it('allows the settings panes to import the backend directly', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'settings', 'ExamplePane.vue');
    const source = [
      '<script setup lang="ts">',
      "import { backendSaveSettings } from '@/services/backend';",
      '</script>',
      '<template><form /></template>',
    ].join('\n');

    expect(boundaryViolationsInSource(source, syntheticPath)).toEqual([]);
  });

  it('rejects a @tauri-apps import even in a settings pane', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'settings', 'ExamplePane.vue');
    const source = [
      '<script setup lang="ts">',
      "import { invoke } from '@tauri-apps/api/core';",
      '</script>',
      '<template><form /></template>',
    ].join('\n');

    expect(boundaryViolationsInSource(source, syntheticPath).map((entry) => entry.kind))
      .toEqual(['tauri']);
  });

  it('rejects a services/backend import from an unguarded-until-now root', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'navigation', 'Example.vue');
    const source = [
      '<script setup lang="ts">',
      "import type { BackendContact } from '@/services/backend';",
      '</script>',
      '<template><nav /></template>',
    ].join('\n');

    expect(boundaryViolationsInSource(source, syntheticPath)).toEqual([
      {
        file: syntheticPath,
        line: 2,
        specifier: '@/services/backend',
        kind: 'backend',
      },
    ]);
  });

  /** ARCH-14 took the last exception out of `components/topics`; it may not come back. */
  it('rejects a backend call reintroduced into a topics component', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'topics', 'TopicDetail.vue');
    const source = [
      '<script setup lang="ts">',
      "import { backendListTopicFiles } from '@/services/backend';",
      '</script>',
      '<template><section /></template>',
    ].join('\n');

    expect(boundaryViolationsInSource(source, syntheticPath)).toEqual([
      {
        file: syntheticPath,
        line: 2,
        specifier: '@/services/backend',
        kind: 'backend',
      },
    ]);
  });

  it('still scans the files the substring gate has to let through', () => {
    // Guards the gate: every listed violation must survive the prefilter.
    const missed = KNOWN_BOUNDARY_VIOLATIONS.filter((entry) => {
      const path = join(REPOSITORY_ROOT, entry.file);
      return boundaryImportsInSource(readFileSync(path, 'utf8'), path).length === 0;
    });

    expect(missed).toEqual([]);
  });
});

describe('local storage boundary', () => {
  it(
    'keeps production access inside the browser storage adapter',
    () => {
      const accesses = productionSources(SOURCE_ROOT)
        .filter((path) => path !== STORAGE_ADAPTER)
        .flatMap(localStorageAccessesInFile)
        .map((entry) => ({
          ...entry,
          file: relative(REPOSITORY_ROOT, entry.file),
        }));
      const details = accesses
        .map((entry) => `${entry.file}:${entry.line} accesses localStorage directly`)
        .join('\n');

      expect(accesses, details || undefined).toEqual([]);
    },
    10_000,
  );

  it('still parses the one file that is allowed to touch storage', () => {
    // Guards the substring gate: the adapter must pass it and be scanned.
    expect(localStorageAccessesInFile(STORAGE_ADAPTER)).not.toEqual([]);
  });

  it('recognizes global, property, element, and destructured access without matching text', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.ts');
    const source = `
      const direct = localStorage.getItem('direct');
      const property = window.localStorage.getItem('property');
      const element = globalThis['localStorage'].getItem('element');
      const { localStorage: storage } = window;
      const { ['localStorage']: computedStorage } = self;
      let assignedStorage: Storage;
      ({ ['localStorage']: assignedStorage } = globalThis);
      const harmless = 'localStorage';
      const mock = { localStorage: new Map() };
      const mockProperty = mock.localStorage;
      function readMock(localStorage: Map<string, string>) {
        return localStorage.get('ignored');
      }
      // localStorage.getItem('ignored');
    `;
    const accesses = localStorageAccessesIn({
      content: source,
      lineOffset: 0,
      scriptKind: ts.ScriptKind.TS,
    }, syntheticPath);

    expect(accesses.map((entry) => entry.line)).toEqual([2, 3, 4, 5, 6, 8]);
  });

  it('recognizes browser globals through transparent TypeScript wrappers', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.ts');
    const source = `
      const asserted = (window as Window).localStorage;
      const typed = (<Window>globalThis)['localStorage'];
      const nonNull = self!.localStorage;
      const constrained = (window satisfies Window).localStorage;
      const { localStorage: destructured } = window as Window;
      let assigned: Storage;
      ({ localStorage: assigned } = globalThis as typeof globalThis);
    `;
    const accesses = localStorageAccessesIn({
      content: source,
      lineOffset: 0,
      scriptKind: ts.ScriptKind.TS,
    }, syntheticPath);

    expect(accesses.map((entry) => entry.line)).toEqual([2, 3, 4, 5, 6, 8]);
  });

  it('recognizes browser globals through transparent TSX wrappers', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.tsx');
    const source = `
      const property = (window as Window).localStorage;
      const element = (globalThis as typeof globalThis)['localStorage'];
      const nonNull = self!.localStorage;
    `;
    const accesses = localStorageAccessesIn({
      content: source,
      lineOffset: 0,
      scriptKind: ts.ScriptKind.TSX,
    }, syntheticPath);

    expect(accesses.map((entry) => entry.line)).toEqual([2, 3, 4]);
  });

  it('recognizes template access at its original line without matching scoped variables', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.vue');
    const source = `
      <output>
        {{ localStorage.getItem('example') }}
      </output>
      <div v-for="localStorage in entries" :key="localStorage">
        {{ localStorage }}
      </div>
    `;
    const accesses = templateLocalStorageAccessesInSource(source, syntheticPath, 10);

    expect(accesses).toEqual([{ file: syntheticPath, line: 13 }]);
  });

  it('recognizes simple, optional, typed, and document template access', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.vue');
    const source = `
      <output>{{ localStorage }}</output>
      <output>{{ window?.localStorage }}</output>
      <output>{{ (window as Window).localStorage }}</output>
      <output>{{ document.defaultView.localStorage }}</output>
      <output v-for="item in localStorage" :key="item">{{ item }}</output>
      <output :[localStorage]="value" />
      <output>{{ window?.[\`localStorage\`] }}</output>
    `;
    const accesses = templateLocalStorageAccessesInSource(source, syntheticPath, 20);

    expect(accesses.map((entry) => entry.line)).toEqual([22, 23, 24, 25, 26, 27, 28]);
  });

  it('does not mistake a script setup binding for browser storage', () => {
    const syntheticPath = join(SOURCE_ROOT, 'components', 'Example.vue');
    const source = `
      <script setup lang="ts">
      const localStorage = new Map<string, string>();
      </script>
      <template>
        <output>{{ localStorage }}</output>
      </template>
    `;

    expect(templateLocalStorageAccessesInSfcSource(source, syntheticPath)).toEqual([]);
  });
});
