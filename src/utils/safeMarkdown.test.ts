import { describe, expect, it } from 'vitest';
import {
  hasMeaningfulMarkdown,
  countRemoteMarkdownImages,
  parseSafeMarkdown,
  safeMarkdownToHtml,
} from './safeMarkdown';

describe('safe Markdown', () => {
  it('detects structural Markdown without treating ordinary prose as Markdown', () => {
    expect(hasMeaningfulMarkdown('Hello team,\nthis is ordinary prose.')).toBe(false);
    expect(hasMeaningfulMarkdown('1. Prefer `intersect` over `retain`.\n2. Keep existing members.')).toBe(true);
  });

  it('preserves lists and inline code in its safe document model', () => {
    const document = parseSafeMarkdown('1. Prefer `intersect`.\n2. Keep existing members.');

    expect(document.blocks).toHaveLength(1);
    expect(document.blocks[0]).toMatchObject({
      kind: 'list',
      ordered: true,
      items: [
        [{ kind: 'paragraph', children: expect.arrayContaining([{ kind: 'code', text: 'intersect' }]) }],
        [{ kind: 'paragraph' }],
      ],
    });
  });

  it('keeps raw HTML inert and never emits Markdown images', () => {
    const input = '<img src=x onerror=alert(1)>\n\n![tracker](https://evil.test/pixel.gif)';
    const html = safeMarkdownToHtml(parseSafeMarkdown(input));

    expect(html).toContain('&lt;img src=x onerror=alert(1)&gt;');
    expect(html).not.toContain('<img');
    expect(html).not.toContain('pixel.gif');
    expect(html).toContain('tracker');
  });

  it('uses declared Markdown image dimensions for a proportional blocked placeholder', () => {
    const document = parseSafeMarkdown(
      '![architecture](https://evil.test/image.png){width=655 height=579}',
    );

    expect(document.blocks[0]).toEqual({
      kind: 'paragraph',
      children: [{
        kind: 'blocked_image',
        alt: 'architecture',
        src: 'https://evil.test/image.png',
        width: 655,
        height: 579,
      }],
    });
    const html = safeMarkdownToHtml(document);
    expect(html).toContain('width:655px;max-width:100%;aspect-ratio:655/579');
    expect(html).not.toContain('evil.test');
    expect(html).not.toContain('{width=');
  });

  it('counts only HTTP(S) Markdown images that can be explicitly released', () => {
    const input = [
      '![first](https://images.example/one.png)',
      '> ![second](http://images.example/two.png)',
      '![unsafe](javascript:alert(1))',
    ].join('\n\n');

    expect(countRemoteMarkdownImages(input)).toBe(2);
  });

  it('resolves GitLab upload paths against the instance linked by the message', () => {
    const input = [
      'Issue created: https://code.example.org/team/project/-/work_items/136',
      '',
      '![diagram](/uploads/0123456789abcdef0123456789abcdef/diagram.png){width=655 height=579}',
    ].join('\n');

    const document = parseSafeMarkdown(input);
    expect(document.blocks[1]).toEqual({
      kind: 'paragraph',
      children: [{
        kind: 'blocked_image',
        alt: 'diagram',
        src: 'https://code.example.org/uploads/0123456789abcdef0123456789abcdef/diagram.png',
        width: 655,
        height: 579,
      }],
    });
    expect(countRemoteMarkdownImages(input)).toBe(1);
  });

  it('does not treat protocol-relative image URLs as same-instance paths', () => {
    const document = parseSafeMarkdown(
      'Issue: https://code.example.org/team/project\n\n![tracker](//tracker.example/pixel.png)',
    );

    expect(document.blocks[1]).toEqual({
      kind: 'paragraph',
      children: [{ kind: 'blocked_image', alt: 'tracker' }],
    });
  });

  it('does not resolve arbitrary root-relative images against a linked site', () => {
    const document = parseSafeMarkdown(
      'Issue: https://code.example.org/team/project\n\n![logo](/assets/logo.png)',
    );

    expect(document.blocks[1]).toEqual({
      kind: 'paragraph',
      children: [{ kind: 'blocked_image', alt: 'logo' }],
    });
  });

  it('keeps invalid image dimension attributes as inert text', () => {
    const document = parseSafeMarkdown(
      '![image](https://evil.test/image.png){width=999999 height=579}',
    );

    expect(document.blocks[0]).toMatchObject({
      kind: 'paragraph',
      children: [
        { kind: 'blocked_image', alt: 'image' },
        { kind: 'text', text: '{width=999999 height=579}' },
      ],
    });
  });

  it('drops unsafe Markdown link targets from generated HTML', () => {
    const html = safeMarkdownToHtml(
      parseSafeMarkdown('[safe](https://example.test) [unsafe](javascript:alert(1))'),
    );

    expect(html).toContain('href="https://example.test/"');
    expect(html).not.toContain('javascript:');
    expect(html).toContain('unsafe');
  });

  it('preserves safe mailto links emitted by the Markdown parser', () => {
    const html = safeMarkdownToHtml(parseSafeMarkdown('Email: mara@example.com'));

    expect(html).toContain('href="mailto:mara@example.com"');
  });

  it('falls back to inert text for unusually large Markdown input', () => {
    const input = `# heading\n${'x'.repeat(250_001)}`;
    const document = parseSafeMarkdown(input);

    expect(hasMeaningfulMarkdown(input)).toBe(false);
    expect(document.blocks).toEqual([{ kind: 'raw_text', text: input }]);
  });
});
