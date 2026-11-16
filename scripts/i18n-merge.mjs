#!/usr/bin/env node
/**
 * Builds the message catalogues from `src/i18n/parts/*.json`.
 *
 * The parts exist so namespaces can be edited independently without one file
 * turning into a merge battlefield. Each part is `{ "en": {...}, "de": {...} }`.
 *
 * Besides the full `en.json`/`de.json` this emits the viewer and tray subsets.
 * Those are derived from the entries' module graphs, so they hold exactly the
 * keys those windows can reach and not one string more — the separate bundles
 * are the reason the viewer still starts in a few kilobytes.
 *
 * The main window loads `startup.<locale>.json` before its first frame; the
 * rest of its keys load as chunks with the lazy code that uses them
 * (`scripts/i18n-split.mjs`, `scripts/vite-i18n-chunks.mjs`). The full
 * catalogues stay for tests and as the source the chunks are cut from.
 */
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import { buildBundles, keyPaths, pick, sortDeep } from './i18n-catalogue.mjs';
import { messageKeys, moduleGraph } from './i18n-graph.mjs';
import { computeSplit, unanchoredKeyCalls } from './i18n-split.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

const NATIVE_UI_FIELDS = {
  tray_show: 'nativeUi.tray.show',
  tray_hide: 'nativeUi.tray.hide',
  tray_quit: 'nativeUi.tray.quit',
  tray_briefing: 'nativeUi.tray.briefing',
  dock_compose: 'nativeUi.dock.compose',
  dock_calendar: 'nativeUi.dock.calendar',
  dock_sync: 'nativeUi.dock.sync',
  dock_settings: 'nativeUi.dock.settings',
  dock_close_other_windows: 'nativeUi.dock.closeOtherWindows',
  notification_archive: 'nativeUi.notifications.archive',
  notification_read: 'nativeUi.notifications.read',
  notification_trash: 'nativeUi.notifications.trash',
  notification_send_failed: 'nativeUi.notifications.sendFailed',
  notification_sync_failed: 'nativeUi.notifications.syncFailed',
  notification_follow_up_due: 'nativeUi.notifications.followUpDue',
  notification_task_due: 'nativeUi.notifications.taskDue',
  notification_no_subject: 'nativeUi.notifications.noSubject',
  notification_channel_name: 'nativeUi.notifications.channelName',
  notification_channel_description: 'nativeUi.notifications.channelDescription',
  notification_new_message: 'nativeUi.notifications.newMessage',
  notification_syncing: 'nativeUi.notifications.syncing',
  notification_reply: 'nativeUi.notifications.reply',
  notification_summary: 'nativeUi.notifications.summary',
  notification_background_sync_channel: 'nativeUi.notifications.backgroundSyncChannel',
  notification_background_sync_description: 'nativeUi.notifications.backgroundSyncDescription',
  notification_background_sync_text: 'nativeUi.notifications.backgroundSyncText',
  notification_action_failed_title: 'nativeUi.notifications.actionFailedTitle',
  notification_action_failed_message: 'nativeUi.notifications.actionFailedMessage',
  notification_action_not_applied_message: 'nativeUi.notifications.actionNotAppliedMessage',
  compose_window_title: 'nativeUi.composeWindowTitle',
  oauth_invalid_title: 'nativeUi.oauth.invalidTitle',
  oauth_invalid_callback: 'nativeUi.oauth.invalidCallback',
  oauth_invalid_state: 'nativeUi.oauth.invalidState',
  oauth_cancelled_title: 'nativeUi.oauth.cancelledTitle',
  oauth_cancelled_message: 'nativeUi.oauth.cancelledMessage',
  oauth_missing_code: 'nativeUi.oauth.missingCode',
  oauth_connected_title: 'nativeUi.oauth.connectedTitle',
  oauth_connected_message: 'nativeUi.oauth.connectedMessage',
  oauth_failed_title: 'nativeUi.oauth.failedTitle',
  oauth_failed_message: 'nativeUi.oauth.failedMessage',
  oauth_close_hint: 'nativeUi.oauth.closeHint',
  oauth_provider_fallback: 'nativeUi.oauth.providerFallback',
};

/**
 * Keys the graph scan cannot see because the call site builds them from a
 * variable. Keep this list short; a static key is always preferable.
 */
export const SUBSETS = {
  viewer: {
    entries: ['src/viewer/main.ts'],
    // Command errors arrive as backend-provided keys. Only the message file
    // commands' own keys ship; anything else falls back to `unexpected`.
    extraKeys: ['backend.errors.unexpected'],
    extraPrefixes: [
      'compose.title',
      'mail.imagePolicy',
      'mail.attachment',
      'shortcuts.actions',
      'swipe',
    ],
    // Rust sources scanned for `backend.errors.*` literals.
    backendSources: ['src-tauri/src/commands/message_file.rs', 'src-tauri/src/commands/reveal.rs'],
  },
  tray: {
    entries: ['src/tray/main.ts'],
    // Command errors arrive as backend-provided keys. Only the tray commands'
    // own keys ship; anything else falls back to `unexpected` at runtime.
    extraKeys: ['backend.errors.unexpected', 'backend.errors.calendarPersistenceFailed'],
    extraPrefixes: [],
    // Rust sources scanned for `backend.errors.*` literals.
    backendSources: ['src-tauri/src/commands/tray.rs'],
  },
};

const BACKEND_KEY_RE = /backend\.errors\.[A-Za-z0-9_-]+/g;

function backendErrorKeys(files) {
  const keys = new Set();
  for (const file of files) {
    const source = readFileSync(path.join(root, file), 'utf8');
    for (const match of source.matchAll(BACKEND_KEY_RE)) keys.add(match[0]);
  }
  return keys;
}

function valueAt(bundle, key) {
  const value = key.split('.').reduce((current, segment) => current?.[segment], bundle);
  if (typeof value !== 'string') throw new Error(`native UI key is not a string: ${key}`);
  return value;
}

function rustNativeFields(bundle) {
  return Object.entries(NATIVE_UI_FIELDS)
    .map(([field, key]) => {
      const value = JSON.stringify(valueAt(bundle, key));
      const line = `        ${field}: ${value}.into(),`;
      if (line.length <= 100) return line;
      const head = `        ${field}: ${value}`;
      if (head.length <= 100) return `${head}\n            .into(),`;
      return `        ${field}:\n            ${value}.into(),`;
    })
    .join('\n');
}

function rustNativeDefaults(bundles) {
  return `// Generated by scripts/i18n-merge.mjs. Do not edit.\n\
use super::UiStrings;\n\n\
pub(super) fn for_language(language: &str) -> UiStrings {\n\
    let primary = language.split(['-', '_']).next().unwrap_or("en");\n\
    if primary.eq_ignore_ascii_case("de") {\n\
        german()\n\
    } else {\n\
        english()\n\
    }\n\
}\n\n\
fn english() -> UiStrings {\n\
    UiStrings {\n\
        lang: "en".into(),\n\
        follow_system_locale: true,\n\
${rustNativeFields(bundles.en)}\n\
    }\n\
}\n\n\
fn german() -> UiStrings {\n\
    UiStrings {\n\
        lang: "de".into(),\n\
        follow_system_locale: true,\n\
${rustNativeFields(bundles.de)}\n\
    }\n\
}\n`;
}

function kotlinString(value) {
  return JSON.stringify(value).replaceAll('$', '\\$');
}

function kotlinNativeDefaults(bundles) {
  const map = (locale) => Object.entries(NATIVE_UI_FIELDS)
    .map(([field, key]) => `    ${kotlinString(field)} to ${kotlinString(valueAt(bundles[locale], key))},`)
    .join('\n');
  return `// Generated by scripts/i18n-merge.mjs. Do not edit.\n\
package de.tbuck.satchel.background\n\n\
import java.util.Locale\n\n\
object NativeUiDefaults {\n\
  private val english = mapOf(\n${map('en')}\n  )\n\
  private val german = mapOf(\n${map('de')}\n  )\n\n\
  fun forSystemLocale(): Map<String, String> =\n\
    if (Locale.getDefault().language == "de") german else english\n\
}\n`;
}

export { buildBundles, keyPaths, pick, sortDeep };

export function subsetKeys(name, allKeys) {
  const spec = SUBSETS[name];
  const referenced = messageKeys(moduleGraph(spec.entries));
  const keys = new Set(spec.extraKeys);
  for (const key of referenced) keys.add(key);
  for (const key of backendErrorKeys(spec.backendSources ?? [])) keys.add(key);
  for (const key of allKeys) {
    // A referenced key may name a subtree ("errors.smtp"); take the leaves.
    for (const seed of referenced) {
      if (key === seed || key.startsWith(`${seed}.`)) keys.add(key);
    }
    for (const prefix of spec.extraPrefixes) {
      if (key === prefix || key.startsWith(`${prefix}.`)) keys.add(key);
    }
  }
  return [...keys].filter((key) => allKeys.has(key)).sort();
}

function main() {
  // `--check` verifies the committed catalogues instead of rewriting them, so
  // CI fails on a part edited without running the build.
  const checkOnly = process.argv.includes('--check');
  const stale = [];
  const emitTarget = (target, content) => {
    if (!checkOnly) {
      writeFileSync(target, content);
      return;
    }
    const current = existsSync(target) ? readFileSync(target, 'utf8') : '';
    if (current !== content) stale.push(path.relative(root, target));
  };
  const emit = (file, content) => emitTarget(path.join(root, 'src', 'i18n', file), content);

  const { bundles, conflicts } = buildBundles();
  const enKeys = new Set(keyPaths(bundles.en));
  const deKeys = new Set(keyPaths(bundles.de));
  const missingDe = [...enKeys].filter((key) => !deKeys.has(key));
  const missingEn = [...deKeys].filter((key) => !enKeys.has(key));

  for (const locale of ['en', 'de']) {
    emit(`${locale}.json`, `${JSON.stringify(sortDeep(bundles[locale]), null, 2)}\n`);
  }

  emitTarget(
    path.join(root, 'src-tauri', 'src', 'commands', 'native_ui_defaults.rs'),
    rustNativeDefaults(bundles),
  );
  emitTarget(
    path.join(
      root,
      'src-tauri',
      'gen',
      'android',
      'app',
      'src',
      'main',
      'java',
      'de',
      'tbuck',
      'satchel',
      'background',
      'NativeUiDefaults.kt',
    ),
    kotlinNativeDefaults(bundles),
  );

  for (const name of Object.keys(SUBSETS)) {
    const keys = subsetKeys(name, enKeys);
    for (const locale of ['en', 'de']) {
      emit(
        `${name}.${locale}.json`,
        `${JSON.stringify(sortDeep(pick(bundles[locale], keys)), null, 2)}\n`,
      );
    }
    process.stdout.write(`subset ${name}: ${keys.length} keys\n`);
  }

  const split = computeSplit(enKeys);
  for (const locale of ['en', 'de']) {
    emit(
      `startup.${locale}.json`,
      `${JSON.stringify(sortDeep(pick(bundles[locale], split.startupKeys)), null, 2)}\n`,
    );
  }
  const lazyKeys = new Set([...split.chunks.values()].flatMap((chunk) => chunk.keys));
  process.stdout.write(
    `startup: ${split.startupKeys.length} keys, lazy chunks: ${lazyKeys.size} keys\n`,
  );
  const unanchored = unanchoredKeyCalls(split.modules);
  if (unanchored.length) {
    process.stderr.write(
      `t()-Aufrufe ohne festen Schlüsselanfang (${unanchored.length}); der Split kann ihre Schlüssel `
        + `nicht zuordnen. Schlüssel mit einem Literal beginnen lassen oder in UNANCHORED_KEYS `
        + `(scripts/i18n-split.mjs) eintragen:\n${unanchored.join('\n')}\n`,
    );
    process.exitCode = 1;
  }

  if (stale.length) {
    process.stderr.write(
      `veraltete Kataloge: ${stale.join(', ')}\nBitte \`pnpm i18n:build\` laufen lassen.\n`,
    );
    process.exitCode = 1;
  }

  process.stdout.write(`merged ${enKeys.size} keys\n`);
  if (conflicts.length) {
    process.stdout.write(
      `conflicting duplicates: ${conflicts.length}\n${conflicts.slice(0, 40).join('\n')}\n`,
    );
  }
  if (missingDe.length) {
    process.stderr.write(`missing in de (${missingDe.length}):\n${missingDe.join('\n')}\n`);
  }
  if (missingEn.length) {
    process.stderr.write(`missing in en (${missingEn.length}):\n${missingEn.join('\n')}\n`);
  }
  if (missingDe.length || missingEn.length) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) main();
