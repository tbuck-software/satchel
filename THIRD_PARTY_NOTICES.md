# Third-party notices

Satchel's own code, artwork, and generic avatar monograms are licensed under
the [MIT License](LICENSE). Third-party material keeps its original license.

## Fonts

The locally bundled fonts are unmodified Google Fonts distributions:

- Figtree, copyright 2022 The Figtree Project Authors, SIL Open Font License 1.1.
  The full notice is in [public/fonts/OFL-figtree.txt](public/fonts/OFL-figtree.txt).
- Fira Code, copyright 2014–2020 The Fira Code Project Authors, SIL Open Font
  License 1.1. The full notice is in [public/fonts/OFL-firacode.txt](public/fonts/OFL-firacode.txt).

The font notices are included in the built frontend alongside the font files.
`scripts/vendor-fonts.mjs` preserves them when refreshing fonts.

## Avatars

The SVG icons for Sentry, Linear, Stripe, DHL, Kickstarter, Product Hunt,
Expedia, Vercel, and X come from Simple Icons under CC0 1.0. The full text is
in [public/avatars/LICENSE.simple-icons.md](public/avatars/LICENSE.simple-icons.md);
individual sources are recorded in [public/avatars/sources.json](public/avatars/sources.json).
Company and product names remain trademarks of their respective owners.
The icons identify senders and do not imply endorsement.

The other bundled avatar images are original generic letter monograms, not
copies of company logos. Downloaded PNG logos and favicons are not distributed.

## Unicode data

The Unicode confusables table retains the notice in
[src/utils/data/UNICODE-LICENSE.txt](src/utils/data/UNICODE-LICENSE.txt).

## Rust and JavaScript dependencies

The vendored lettre fork retains its MIT license in [vendor/lettre/LICENSE](vendor/lettre/LICENSE)
and documents changes in [vendor/lettre/SATCHEL_PATCH.md](vendor/lettre/SATCHEL_PATCH.md).
postroom is Apache-2.0 licensed. Other dependencies retain the licenses declared
in their packages. CI checks the Rust dependency license policy with cargo-deny.
Release archives include the complete notices generated from the locked Rust
and JavaScript dependency graphs.
