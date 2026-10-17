import { describe, expect, it } from 'vitest';
import {
  darkModeBackgroundColor,
  isDesignedEmailMarkup,
  readableDarkModeTextColor,
  responsiveEmailTableWidth,
  rewriteCssColorsForDark,
} from './emailAppearance';

describe('email appearance', () => {
  it('does not mistake width-constrained signature images for a designed newsletter', () => {
    expect(isDesignedEmailMarkup('<p>Hello</p><img style="width:1.9in" width="187">')).toBe(false);
    expect(isDesignedEmailMarkup('<table role="presentation"><tr><td>Hello</td></tr></table>')).toBe(true);
  });

  it('raises low-contrast text colors for a dark canvas', () => {
    expect(readableDarkModeTextColor('#3A3A3A')).not.toBe('#3a3a3a');
    expect(readableDarkModeTextColor('blue')).toMatch(/^#[0-9a-f]{6}$/);
    expect(darkModeBackgroundColor('white')).toBe('var(--satchel-email-canvas)');
  });

  it('rewrites simple stylesheet colors without touching unrelated declarations', () => {
    const output = rewriteCssColorsForDark('.reply { color: blue; font-weight: 600; background:#fff }');
    expect(output).not.toContain('color: blue');
    expect(output).toContain('font-weight: 600');
    expect(output).toContain('background: var(--satchel-email-canvas)');
  });

  it('maps neutral sender canvases to the reader while retaining coloured panels', () => {
    for (const color of ['#f4f4f4', 'rgb(238, 238, 238)', '#202124']) {
      expect(darkModeBackgroundColor(color)).toBe('var(--satchel-email-canvas)');
    }
    expect(darkModeBackgroundColor('#174c70')).toBe('#174c70');
    expect(darkModeBackgroundColor('transparent')).toBeNull();
  });

  it('makes fixed email table widths responsive without changing fluid widths', () => {
    expect(responsiveEmailTableWidth('570', '570px')).toBe('min(100%, 570px)');
    expect(responsiveEmailTableWidth('570', '100%')).toBeNull();
    expect(responsiveEmailTableWidth('640', '')).toBe('min(100%, 640px)');
    expect(responsiveEmailTableWidth(null, 'calc(100% - 24px)')).toBeNull();
  });

});
