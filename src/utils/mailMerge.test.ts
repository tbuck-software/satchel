import { describe, expect, it } from 'vitest';
import { contactsMergeSource, hasMergeToken, mergeDurationMinutes, mergeToken, splitDisplayName } from './mailMerge';

describe('mail merge helpers', () => {
  it('splits display names into first and last name', () => {
    expect(splitDisplayName('Anna Meier')).toEqual({ first: 'Anna', last: 'Meier' });
    expect(splitDisplayName('Anna Maria  Meier')).toEqual({ first: 'Anna Maria', last: 'Meier' });
    expect(splitDisplayName('Meier, Anna')).toEqual({ first: 'Anna', last: 'Meier' });
    expect(splitDisplayName('Cher')).toEqual({ first: 'Cher', last: '' });
    expect(splitDisplayName('anna@example.com')).toEqual({ first: '', last: '' });
  });

  it('turns contacts into rows with the contact fields', () => {
    const source = contactsMergeSource([
      { email: 'anna@example.com', name: 'Anna Meier' },
      { email: 'ben@example.com', name: 'ben@example.com' },
    ], '2 contacts');
    expect(source.emailField).toBe('Email');
    expect(source.fields).toEqual(['FirstName', 'LastName', 'Name', 'Email']);
    expect(source.rows[0]!.values).toEqual({ FirstName: 'Anna', LastName: 'Meier', Name: 'Anna Meier', Email: 'anna@example.com' });
    // An address as the name is no name: the placeholder stays empty.
    expect(source.rows[1]!.values.FirstName).toBe('');
  });

  it('builds and recognises tokens', () => {
    expect(mergeToken('FirstName')).toBe('{{FirstName}}');
    expect(hasMergeToken('Hi {{ FirstName|there }}')).toBe(true);
    expect(hasMergeToken('Hi {{}}')).toBe(false);
  });

  it('estimates the run time from the pace', () => {
    expect(mergeDurationMinutes(1, 15)).toBe(0);
    expect(mergeDurationMinutes(5, 15)).toBe(1);
    expect(mergeDurationMinutes(500, 15)).toBe(125);
  });
});
