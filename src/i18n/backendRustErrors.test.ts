import { readdirSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const COMMAND_DIR = 'src-tauri/src/commands';
const TAURI_COMMAND_FILES = readdirSync(COMMAND_DIR)
  .filter((file) => file.endsWith('.rs'))
  .map((file) => `${COMMAND_DIR}/${file}`);
const COMMAND_ERROR_FILES = [
  ...TAURI_COMMAND_FILES,
  'src-tauri/src/sync/mailbox_sync.rs',
  'src-tauri/src/sync/message_ingest.rs',
  'src-tauri/src/sync/outbox.rs',
  'src-tauri/src/sync/workflow_executor.rs',
  'crates/satchel-core/src/sync/remote_mutations.rs',
];

const GERMAN_ERROR_WORD = /(?:[äöüÄÖÜß]|\b(?:bereits|bitte|darf|enthält|erneut|für|kann|keine|nicht|noch|wurde)\b)/u;
const RUST_STRING = /"((?:\\.|[^"\\])*)"/g;

function productionSource(source: string): string {
  const withoutTests = source.split('#[cfg(test)]', 1)[0] ?? source;
  return withoutTests
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '')
    .replace(/^.*(?:tracing|log)::\w+!.*$/gm, '');
}

function germanErrorLiterals(source: string): string[] {
  const findings: string[] = [];
  const production = productionSource(source);
  const errorExpression = /(?:Err|ok_or(?:_else)?|map_err)\s*\(([\s\S]{0,600}?)(?:\)(?:\?|;|,)|$)/g;
  for (const expression of production.matchAll(errorExpression)) {
    for (const literal of expression[1]?.matchAll(RUST_STRING) ?? []) {
      if (GERMAN_ERROR_WORD.test(literal[1] ?? '')) findings.push(literal[1] ?? '');
    }
  }
  return findings;
}

const INFALLIBLE_COMMANDS = new Map<string, string>([
  ['app_build_info', 'BuildInfo'],
  // Sets a flag the running import reads between messages. Whether an import
  // is running at all is not this command's business to report.
  ['cancel_import', '()'],
  // Only removes the authenticated caller from a pending shutdown round.
  ['acknowledge_draft_flush', '()'],
  ['get_source_capabilities', 'std::collections::HashMap<String, SourceCapabilities>'],
  ['pending_crash_reports', 'Vec<CrashEntry>'],
  ['record_frontend_diagnostic', '()'],
  ['runtime_capabilities', 'RuntimeCapabilitiesDto'],
  ['stop_inline_image_backfill', '()'],
  // Reads a buffer the shell filled before any webview existed; there is no
  // failure mode to report.
  ['take_pending_open_with', 'Vec<String>'],
  ['take_pending_notification_thread', 'Option<String>'],
]);

function invalidCommandErrorContracts(source: string): Array<{ name: string; result: string }> {
  const findings: Array<{ name: string; result: string }> = [];
  const signature = /#\[tauri::command(?:\([^\]]*\))?\][\s\S]{0,2000}?\bfn\s+(\w+)[\s\S]{0,1600}?\)(?:\s*->\s*([\s\S]*?))?\s*\{/g;
  for (const match of productionSource(source).matchAll(signature)) {
    const name = match[1] ?? '';
    const result = (match[2] ?? '()').replace(/\s+/g, ' ').trim();
    const infallibleResult = INFALLIBLE_COMMANDS.get(name);
    if (infallibleResult ? result !== infallibleResult : !result.startsWith('CommandResult<')) {
      findings.push({ name, result });
    }
  }
  return findings;
}

describe('backend error localization gate', () => {
  it('uses the structured error contract at every fallible Tauri command boundary', () => {
    const findings = TAURI_COMMAND_FILES.flatMap((file) =>
      invalidCommandErrorContracts(readFileSync(file, 'utf8')).map((finding) => ({
        file,
        ...finding,
      })),
    );

    expect(findings).toEqual([]);
  });

  it('keeps command-facing Rust errors out of the UI language', () => {
    const findings = COMMAND_ERROR_FILES.flatMap((file) =>
      germanErrorLiterals(readFileSync(file, 'utf8')).map((text) => ({ file, text })),
    );

    expect(findings).toEqual([]);
  });

  it('ignores fixtures, comments and logs but catches returned German text', () => {
    const source = `
      // return Err("Bitte erneut versuchen.");
      tracing::warn!("Der Vorgang läuft bereits.");
      fn command() -> Result<(), String> {
        return Err("Diese Aktion kann nicht ausgeführt werden.".to_string());
      }
      #[cfg(test)]
      mod tests {
        const FIXTURE: &str = "Der Ordner enthält Nachrichten.";
      }
    `;

    expect(germanErrorLiterals(source)).toEqual([
      'Diese Aktion kann nicht ausgeführt werden.',
    ]);
  });
});
