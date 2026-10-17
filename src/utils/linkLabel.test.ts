// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { visibleLinkText } from './linkLabel';

function parsedAnchor(html: string): Element {
  return new DOMParser().parseFromString(html, 'text/html').querySelector('a')!;
}

function renderedAnchor(html: string): Element {
  document.body.innerHTML = html;
  return document.querySelector('a')!;
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('visibleLinkText', () => {
  it.each([
    ['display:none', '<a href="#">paypal.com<span style="display:none">!</span></a>'],
    ['visibility:hidden', '<a href="#">paypal.com<span style="visibility:hidden">x</span></a>'],
    ['font-size:0', '<a href="#">paypal<b style="font-size:0"> </b>.com</a>'],
    ['opacity:0', '<a href="#">paypal.com<i style="opacity:0">/x</i></a>'],
    ['the hidden attribute', '<a href="#">paypal.com<span hidden>!</span></a>'],
  ])('leaves out text hidden by %s in a parsed document', (_, html) => {
    expect(visibleLinkText(parsedAnchor(html))).toBe('paypal.com');
  });

  it('keeps text a nested element makes visible again', () => {
    const html = '<a href="#"><span style="visibility:hidden">x<b style="visibility:visible">paypal.com</b></span></a>';
    expect(visibleLinkText(parsedAnchor(html))).toBe('paypal.com');
  });

  it('asks the computed style in a rendered document, so class rules count too', () => {
    const anchor = renderedAnchor(
      '<style>.gone { display: none }</style><a href="#">paypal.com<span class="gone">!</span></a>',
    );
    expect(visibleLinkText(anchor)).toBe('paypal.com');
  });

  it('reads a line break as a space', () => {
    expect(visibleLinkText(parsedAnchor('<a href="#">Sign in<br>paypal.com</a>'))).toBe('Sign in paypal.com');
  });
});
