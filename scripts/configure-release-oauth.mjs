// @ts-check
import { appendFileSync } from 'node:fs';

// Google's downloaded desktop client JSON distinguishes installed/public clients
// from confidential web clients. Never infer that distinction from a key prefix.
try {
  const config = JSON.parse(process.env.SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG || 'null');
  const client = config?.installed;
  if (!client || config.web || client.token_uri !== 'https://oauth2.googleapis.com/token') {
    throw new Error('Invalid desktop registration');
  }
  const values = {
    SATCHEL_GMAIL_CLIENT_ID: client.client_id,
    SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET: client.client_secret,
  };
  if (!values.SATCHEL_GMAIL_CLIENT_ID?.endsWith('.apps.googleusercontent.com')
      || Object.values(values).some(value => typeof value !== 'string' || !value.trim() || /[\r\n\0]/.test(value))) {
    throw new Error('Invalid desktop registration');
  }
  if (!process.env.GITHUB_ENV) throw new Error('Missing GitHub environment file');
  appendFileSync(process.env.GITHUB_ENV, Object.entries(values).map(([key, value]) => `${key}=${value.trim()}\n`).join(''));
} catch {
  // Parser errors can contain credential values. Report only a fixed message.
  console.error('A Google Desktop app client JSON is required in SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG. Web clients are not supported.');
  process.exitCode = 1;
}
