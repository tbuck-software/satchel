import { describe, expect, it } from 'vitest';
import { cleanContactName, contactFallbackName, contactLabel, isContactEmail } from './contactIdentity';

describe('contact identities', () => {
  it('rejects inline-image artifacts while keeping service and unusual valid mailboxes', () => {
    for (const email of ['logo@2x.png', 'data:image/png;base64,abc@def.png', 'cid:logo@image.test', 'a@@example.test', 'broken', '@example.test']) {
      expect(isContactEmail(email), email).toBe(false);
    }
    for (const email of ['noreply@3example.test', '0123456789abcdef0123456789abcdef@group.calendar.google.com', 'person+tag@example.test', 'name@example.zip', 'a/b@example.test']) {
      expect(isContactEmail(email), email).toBe(true);
    }
  });

  it('cleans escaped outer phrases without removing quotes inside a name', () => {
    expect(cleanContactName(String.raw`\\"Morgan, Alex\\"" (via Mailing List)`)).toBe('Morgan, Alex (via Mailing List)');
    expect(cleanContactName('"Meier, Anna"')).toBe('Meier, Anna');
    expect(cleanContactName('John "Jack" Doe')).toBe('John "Jack" Doe');
    expect(cleanContactName('(über Google Docs)')).toBe('');
    expect(cleanContactName('Alex Morgan (über Google Docs)')).toBe('Alex Morgan (über Google Docs)');
  });

  it('uses the service domain for opaque hexadecimal local parts', () => {
    expect(contactFallbackName('0123456789abcdef0123456789abcdef@group.calendar.google.com')).toBe('group.calendar.google.com');
    expect(contactFallbackName('plain@example.test')).toBe('plain');
    expect(contactLabel('PLAIN@example.test', 'plain@example.test')).toBe('plain');
  });
});
