# Satchel Style Guide

## Design Philosophy
A mail client that reads like paper. The light palette is warm paper and the dark palette is cool charcoal. The
window is three panes deep: the sidebar sits back, the thread list steps forward, the reading
canvas is the brightest sheet and carries the message without a card around it.
The primary accent is the petrol from the inbox reference; everything else is paper,
ink and a hairline. Feels like a premium tool — not decorative, but precise.

The light theme follows the drawing. The dark theme uses nearly black surfaces
with a slight blue bias. The sidebar is darkest and the reader brightest, so
components keep the same surface roles in both themes. Dark backgrounds, text
and borders contain no warm yellow or olive tint.

---

## What is enforced

`src/architecture/designSystem.test.ts` runs in `vitest`, so in `make check` and
in CI. It reads `src/components/`, `src/views/`, `src/viewer/` and `src/tray/`
and fails on:

| Rule | What it catches |
|------|-----------------|
| No raw colours | `#rrggbb`, `rgb()`, `rgba()`, and a declaration whose whole value is `white` or `black`, outside the exceptions listed below |
| No Tailwind default palette | `text-slate-400`, `text-white`, `bg-white/[0.06]` — only the classes bridged in `tailwind.css` |
| UI comes from the kit | a hand-rolled `<button>` outside `src/components/ui/` |
| Icon buttons carry a name | an icon-only button in the kit without `aria-label` |
| Radii and type sizes from the scale | literal `border-radius:` or `font-size:`, `text-[13px]`, `font-[550]` |
| Icons share one frame | an `<svg>` with `stroke-width="1.4"`, a non-square or off-scale `viewBox`, or a render size of 13/15/17/19px |
| Transitions come from the motion tokens | a literal time in `transition:` or `transition-duration:` |
| The focus ring stays on | `outline: none`, `outline: 0`, `outline-none` without a listed replacement |
| Contrast | a rule that declares both its own background and a `--text-*` colour, measured against the resolved tokens in **both** themes: 4.5 : 1 for every text token, tertiary included. Each text token must also clear 4.5 : 1 on `--bg-base`, `--bg-list`, `--bg-surface`, `--bg-raised`, `--bg-overlay`, `--bg-hover` and `--bg-active` |

The tree does not pass these rules yet. Today's breakage is frozen file by file
in that test's `KNOWN_*` lists, each entry with a count and a reason, and each
rule has a budget that may only fall. A new violation therefore costs two
deliberate edits — the entry and a raised budget — and a listed count that has
grown too high fails as well, so a list can only get shorter. UI-07 and UI-11
empty the two long ones.

`src/styles/themeContrast.test.ts` is the narrow sibling: it measures four
fixed surfaces that broke in the light theme once and must not break again.
`src/styles/motion.test.ts` guards the global `prefers-reduced-motion`
fallback, which is four declarations nobody would miss by looking.
`src/styles/tailwindBridge.test.ts` guards the other direction: every rung of
the type and spacing scales has to reach Tailwind, or a class and its token
drift apart again.

Not machine-checked, and therefore only as good as review: spacing (`--space-*`),
the motion tokens, and everything under *Perceived Performance* and
*Error States*.

---

## Color Palette

### Surfaces
Three steps of paper, plus one well below them. The direction flips with the
theme; the roles do not.

| Token             | Light (paper) | Dark      | Usage                                    |
|-------------------|-----------|-----------|------------------------------------------|
| `--bg-base`       | `#f4f2ed` | `#111214` | App background and sidebar — the pane that sits back |
| `--bg-list`       | `#faf9f6` | `#17181b` | The thread-list pane, one step forward   |
| `--bg-surface`    | `#ffffff` | `#1c1d21` | The reading canvas, and cards on `--bg-base` |
| `--bg-raised`     | `#ebe8e1` | `#24262b` | Quiet buttons, pills, search fields      |
| `--bg-stack`      | `#f1eee8` | `#1a1b1f` | Collections, distinct from row hover     |
| `--bg-stack-hover`| `#e9e5dc` | `#22242a` | Hover feedback on collections           |
| `--bg-overlay`    | `#ffffff` | `#2e3137` | Tooltips, overlays, popovers             |
| `--bg-inset`      | `#ebe8e1` | `#0c0d0f` | A well *below* the pane: code blocks, swipe tracks |
| `--bg-sidebar`    | `#f4f2ed` | `#111214` | Sidebar background (same as base)        |
| `--bg-backdrop`   | `rgba(28, 27, 25, 0.35)` | `rgba(5, 7, 11, 0.55)` | The scrim behind a modal — the only one |
| `--bg-hover`      | `#efece6` | `#23252a` | Hover state for interactive elements     |
| `--bg-active`     | `#dcecef` | `#213247` | Passive selected-state background        |
| `--bg-active-hover`| `#cce2e6`| `#2b405a` | Active + hover combined                  |

On paper the reader is the *brightest* surface, not an inset one: the message is
the sheet, so it gets no card of its own. `--bg-inset` kept its name and lost
that job — it is the recessed fill now, and reaching for it to draw a pane
gives a well where the reading canvas should be.

Active navigation items are a raised sheet — `--bg-surface` with `--shadow-sm` —
not a solid accent pill. The accent stays for counts, the primary button and
the unread dot, so the one saturated colour in the window still means something.

### Text
| Token              | Light     | Dark      | Usage                           |
|--------------------|-----------|-----------|---------------------------------|
| `--text-primary`   | `#1c1b19` | `#e9edf3` | Headlines, primary text         |
| `--text-secondary` | `#4f4b44` | `#a6aebc` | Body text, labels               |
| `--text-tertiary`  | `#6a655b` | `#939cab` | Timestamps, hints, disabled     |
| `--text-inverse`   | `#faf9f6` | `#111214` | Text on accent backgrounds      |

### Borders
| Token                | Light     | Dark      | Usage                         |
|----------------------|-----------|-----------|-------------------------------|
| `--border-color`     | `#ddd8cd` | `#30333a` | Primary borders               |
| `--border-color-muted`| `#e6e2db`| `#272a30` | Subtle separators             |

### Accent Colors
The primary is the petrol from the inbox reference (`#17707f`), lifted to `#277e8b`
in the dark theme while keeping white button text above 4.5 : 1. The state hues turn warm in the light
theme — the bright set below is measured against a near-black and sits on paper
like stickers.

| Token            | Light     | Dark      | Usage                             |
|------------------|-----------|-----------|-----------------------------------|
| `--accent-primary`  | `#17707f` | `#277e8b` | Primary actions, links, focus  |
| `--accent-green` | `#2f7d3b` | `#3fb950` | Success, done states              |
| `--accent-amber` | `#b7791f` | `#d29922` | Warnings, todo states             |
| `--accent-red`   | `#c0392f` | `#f85149` | Danger, delete, high priority     |
| `--accent-primary-hover` | mixed | The petrol fill under the pointer — **lighter** in the dark theme, **darker** in the light one |
| `--accent-red-hover`  | mixed | The danger fill under the pointer — darker in both themes: white ink on a lightened `#f85149` measures 2.9 : 1 |
| `--accent-purple`| `#7c4dab` | `#a371f7` | Snoozed, deferred              |
| `--accent-cyan`  | `#10707c` | `#39d2c0` | Waiting states                 |
| `--accent-pink`  | `#b4485d` | `#f778ba` | Decorative accent              |
| `--accent-orange`| `#b5551d` | `#db6d28` | Secondary warning              |
| `--accent-yellow`| `#b7791f` | `#fbbf24` | Star / favourite iconography   |

### Softer Accents
Body text in a saturated accent vibrates against a neutral pane. These darken in
the light theme so they keep their contrast.

| Token                | Dark      | Light     | Usage                     |
|----------------------|-----------|-----------|---------------------------|
| `--accent-red-soft`  | `#ff8b8b` | `#a5302a` | Error and failure text    |
| `--accent-amber-soft`| `#f3bf5b` | `#7d5310` | Warning and caution text  |

### Theme-independent Fills
One value in both themes, on purpose — each sits on something that is not a
themed surface.

| Token             | Value     | Usage                                       |
|-------------------|-----------|---------------------------------------------|
| `--text-on-accent`| `#ffffff` | Text and knobs on a saturated accent fill; `text-on-accent` in Tailwind |
| `--text-on-amber` | `#17130c` | Dark ink on an amber fill, where white fails|
| `--email-canvas`  | `#ffffff` | Paper behind a message — the mail brings its own colours |
| `--swipe-blue`    | `#1688ee` | Swipe-action track                          |
| `--swipe-green`   | `#82cc31` | Swipe-action track                          |
| `--swipe-amber`   | `#c98516` | Swipe-action track                          |
| `--swipe-cyan`    | `#188fa3` | Swipe-action track                          |
| `--swipe-red`     | `#dc4545` | Swipe-action track                          |
| `--lightbox-scrim`| `rgb(8 10 14 / 88%)` | The near-black behind a full-screen image |
| `--lightbox-chrome`| `rgb(35 38 45 / 88%)` | The close button on that scrim          |
| `--lightbox-ink`  | `rgb(242 244 248 / 88%)` | Caption and close glyph on that scrim |
| `--lightbox-shadow`| `0 20px 70px rgb(0 0 0 / 45%)` | The vignette that lifts the photo off the scrim |

The lightbox is the one surface that does not follow the theme. A picture
viewer is dark on purpose — `--bg-backdrop` is 35% ink in the light theme and
does not carry a photograph — so it gets its own four values instead of seven
`rgb()` literals in `SafeMarkdownBody.vue`. Its close button is
`IconButton size="lg"`; only the colours are overridden.

`color: white` is a bug for the same reason a hex literal is, one step
further down: it is not wrong today — `--text-on-accent` **is** `#ffffff` — but
it does not say whether the white was chosen because the fill underneath is
blue or because nobody looked. Mixing *toward* a keyword stays allowed
(`color-mix(in srgb, var(--border-color) 84%, white 7%)` lifts a border by a
measured amount); a whole value of `white` or `black` does not.

Hex literals outside `tokens.css` are a bug, with four documented exceptions:
values that leave the stylesheet (outgoing mail HTML in `RichTextEditor.vue` and
`utils/emailAppearance.ts`), CSS injected into the mail iframe `srcdoc`
(`IsolatedEmailDocument.vue`, which receives the resolved reader canvas and ink
across the iframe boundary),
paper colours in `@media print`, and identity palettes hashed per sender or
account, which must not shift with the theme.

The sidebar's compact compose button uses `--accent-primary` with
`--text-on-accent`. Pressed filter pills use `--action-neutral` with
`--action-neutral-ink`. Sender initials use a 16% identity tint on `--bg-surface`, with
ink mixed from the identity colour and `--text-primary`.

Stacks use up to three offset sender avatars, the count next to the sender,
a preview of distinct subjects, and a translated collection summary. Their
resting surface is `--bg-stack`, never the button fill `--bg-raised`. Mail
rows reserve 72px through `THREAD_HEIGHT` in `ThreadGroupList.vue`; stack
content is inset inside the same slot.

Unread mail has one petrol dot attached to the avatar, including stacks. Priority
does not draw a ring around the sender, and account colours do not add a second
avatar badge.

The adapted HTML view uses the reader's `--bg-surface` for document and neutral
sender backgrounds. Coloured content panels keep their adapted fill. Original
HTML keeps sender colours on `--email-canvas`.

---

## Typography

### Font Stack
- **UI (sans-serif):** `Figtree, system-ui, -apple-system, sans-serif`
- **Monospace:** `'Fira Code', 'SF Mono', monospace`

### Size Scale
| Token              | Size  | Tailwind    | Usage                      |
|--------------------|-------|-------------|----------------------------|
| `--font-size-3xs`  | 10px  | `text-3xs`  | Dense counters             |
| `--font-size-2xs`  | 11px  | `text-2xs`  | Badges, status indicators  |
| `--font-size-xs`   | 12px  | `text-xs`   | Timestamps, counters       |
| `--font-size-sm`   | 13px  | `text-sm`   | Secondary labels, snippets |
| `--font-size-md`   | 14px  | `text-md`   | Body text, default         |
| `--font-size-lg`   | 15px  | `text-lg`   | Slightly emphasized text   |
| `--font-size-xl`   | 17px  | `text-xl`   | Section titles             |
| `--font-size-2xl`  | 22px  | `text-2xl`  | Page/thread headlines      |

`tailwind.css` bridges the two columns onto each other, so `text-sm` and
`var(--font-size-sm)` are one value. Take the class or the token, never a pixel:
`tokens.css` lifts the whole scale under 1020px (13px body becomes 15px, 11px
labels 12px) and a hard-coded size sits out the lift — which is how the phone
ended up reading a thread row in two sizes at once.

### Weights
| Token               | Weight | Tailwind        | Usage                              |
|---------------------|--------|-----------------|------------------------------------|
| `--weight-regular`  | 400    | `font-normal`   | Body text                          |
| `--weight-medium`   | 500    | `font-medium`   | Labels, secondary emphasis         |
| `--weight-semibold` | 600    | `font-semibold` | Active items, buttons              |
| `--weight-strong`   | 650    | `font-strong`   | List titles, small caps labels     |
| `--weight-bold`     | 700    | `font-bold`     | Headlines, brand text              |

Decided in UI-11: five steps, no others. Figtree is vendored as one variable
face per subset (`wght@400..700`), so 650 is not — it is used on purpose for titles that must stand out from 600 without
shouting like 700, and now it has a name. 550, 620, 680, 750, 760 and 800 were
the same intentions spelled differently and went to the nearest step.

---

## Spacing
| Token          | Size | Tailwind | Usage                              |
|----------------|------|----------|------------------------------------|
| `--space-2xs`  | 2px  | `0.5`    | Inline gaps                        |
| `--space-xs`   | 4px  | `1`      | Tight padding                      |
| `--space-1-5`  | 6px  | `1.5`    | Icon-to-label gap, compact padding |
| `--space-sm`   | 8px  | `2`      | Component padding                  |
| `--space-2-5`  | 10px | `2.5`    | Row padding, menu and nav rows     |
| `--space-md`   | 12px | `3`      | Section gaps                       |
| `--space-3-5`  | 14px | `3.5`    | Header padding                     |
| `--space-lg`   | 16px | `4`      | Panel padding                      |
| `--space-5`    | 20px | `5`      | Card padding                       |
| `--space-xl`   | 24px | `6`      | Major sections                     |
| `--space-2xl`  | 32px | `8`      | Page-level spacing                 |

Decided in UI-11: a 2px grid up to 16, then 20/24/32. The alternative — folding
everything onto 8/12/16 — would have moved hundreds of rows by a few pixels for
no gain; 10, 6 and 14px were the second, fourth and sixth most used values.
Named rungs keep their names; the half steps carry the Tailwind rung, so
`p-2.5` and `var(--space-2-5)` read as the same thing. An odd pixel rounds down.

`--spacing` is pinned to 4px in `tailwind.css`, so `gap-3` and `--space-md` are
the same 12px.

---

## Border Radius
| Token          | Size   | Usage                       |
|----------------|--------|-----------------------------|
| `--radius-xs`  | 5px    | Badges, small chips         |
| `--radius-sm`  | 8px    | Buttons, inputs, nav rows   |
| `--radius-md`  | 12px   | List rows, cards, panels    |
| `--radius-lg`  | 16px   | Modals, large panels        |
| `--radius-full`| 999px  | Avatars, pill buttons       |

Literals go to the nearest step: 4–7px `xs`, 8–11px `sm`, 12–15px `md`,
16–24px `lg`, 99px and up `full`. `50%` (a circle) and `0` are shapes, not steps.

---

## Shadows
| Token          | Light                                    | Dark                          | Usage              |
|----------------|------------------------------------------|-------------------------------|--------------------|
| `--shadow-sm`  | `0 1px 3px …0.10`, `0 0 0 1px …0.05`     | `0 2px 8px …0.25` + hairline  | A raised sheet: selected row, active nav item |
| `--shadow-md`  | `0 8px 24px …0.10` + hairline            | `0 8px 24px …0.35`            | Dropdowns, cards   |
| `--shadow-lg`  | `0 16px 48px …0.14` + hairline           | `0 16px 48px …0.45`           | Modals, overlays   |
| `--shadow-glow`| `0 0 20px …0.10`                         | `0 0 20px …0.15`              | Focus glow         |

All three light-theme shadows carry a hairline ring of their own. A white sheet
on pale paper is a 1.07 : 1 step, and a drop shadow draws no top edge — without
the ring a row looks cut off along the top, and a menu opened over the reading
canvas (also white) has no edge at all.

---

## Focus

One ring, two shapes. `base.css` puts it on every `button`, `input`, `textarea`,
`select` and `a`; a component that wants focus feedback either inherits it or
reuses the same tokens. A component-local ring of its own is a bug.

| Token                    | Value                                                            | Usage                                  |
|--------------------------|------------------------------------------------------------------|----------------------------------------|
| `--border-focus`         | `2px solid var(--accent-primary)`                                   | The ring, as an outline                |
| `--focus-outline-offset` | `1px`                                                            | The gap that goes with it              |
| `--focus-ring`           | `0 0 0 2px color-mix(in srgb, var(--accent-primary) 40%, transparent)` | The same ring as a shadow            |

Reach for `--focus-ring` only where an outline cannot do the job: the focused
element is not the painted one (a visually hidden input next to its track), or
an outline would be clipped. `AppButton`, `IconButton` and `NavItem` define no
focus of their own on purpose.

Switching the ring off needs a replacement, and the check lists the places that
draw one elsewhere.

---

## Motion
| Token              | Value   | Usage                      |
|--------------------|---------|----------------------------|
| `--duration-fast`  | 100ms   | Hover, micro-interactions  |
| `--duration-normal`| 180ms   | Transitions, fades         |
| `--duration-slow`  | 280ms   | Overlays, row exits, modals|
| `--duration-spatial`| 320ms  | One large spatial move: docking the composer |
| `--ease-out`       | cubic-bezier(0.16, 1, 0.3, 1) | Default easing    |
| `--ease-in-out`    | cubic-bezier(0.65, 0, 0.35, 1) | Symmetric easing  |

Use the tokens instead of component-local timings: a literal in a `transition`
or `transition-duration` inside `src/components/`, `src/views/`, `src/viewer/`
or `src/tray/` fails the check. State feedback should begin immediately even
when the accompanying motion takes longer.

Keyframe animations are out of that rule. A skeleton shimmer, a spinner and a
scan line are loops with their own rhythm, and three interface-transition steps
do not describe them; the one-shot animations (row exit, done cue) do use the
tokens.

All non-essential motion must honor `prefers-reduced-motion`. `base.css`
carries the fallback for everything — 1ms rather than 0, because a transition
that never runs never fires `transitionend` and several rows wait for it. A
component adds a local rule only when reduced motion should mean something
other than "faster": `UndoFab` freezes its countdown ring and puts the time in
the text instead. Repeating the fallback in a component is not that, and
`src/styles/motion.test.ts` counts the files that have a reason.

---

## Perceived Performance

### Loading feedback

- Do not show transient loading UI immediately. The default grace period is
  160ms; an operation that completes inside it should appear instantaneous.
- Local search uses a 120ms debounce as the baseline. Longer debounce windows
  need evidence that they materially reduce work without making typing feel
  delayed.
- During a background refresh, keep the last valid content visible, mark its
  region with `aria-busy`, and use a progress treatment that does not consume
  additional layout space.
- When no prior content exists, use a skeleton that approximates the final row
  or pane geometry. Use a blocking spinner only when the user cannot continue
  and no representative layout can be shown.
- Pulse a skeleton container rather than animating every placeholder bar.
  Multiple independently animated bars create unnecessary compositor work.

### Stable state and geometry

- Account-, mailbox-, contact-, and filter-scoped results are valid only for
  the scope that produced them. Invalidate them synchronously on navigation so
  content from the previous scope never flashes in the destination view.
- Reserve space for conditional icons and activity indicators. Their presence
  must not resize text inputs, toolbars, rows, or dialogs.
- Keep overlay dimensions predictable while their content changes. Scrollable
  result regions should use `scrollbar-gutter: stable` where a scrollbar could
  otherwise shift the layout.
- Heavy, variable-height content well outside a scroll viewport may use
  `content-visibility: auto` together with a representative
  `contain-intrinsic-block-size`. Never use it without the intrinsic fallback,
  because an unreserved offscreen subtree can destabilize the scroll position.
- First paint must not wait for notification bridges, aggregate metadata, or
  uncommon UI surfaces. Run independent startup work concurrently, lazy-load
  uncommon surfaces, and prefetch likely next interactions only after the main
  view is usable.

---

## Layout Principles

### Window Chrome
- Custom titlebar — no native OS title bar
- macOS: traffic lights at top-left of sidebar; sidebar header has `data-tauri-drag-region`
- Windows/Linux: custom close/min/max buttons at top-right of header
- Sidebar header area doubles as drag region

### Sidebar (left)
- Width: 220–420px, resizable
- Contains: brand/logo, search with a mailbox-filter popover and compact compose
  button, primary nav, mailbox tree, settings footer
- Workflow navigation: Inbox, Snoozed, Set aside, Outbox, Done
- Native macOS places the brand in the traffic-light row with a clear control
  inset. Browser and other platforms keep a separate brand header.
- Active state: solid accent-primary pill with white text (NavItem)

### Thread List (center column)
- Width: 320–760px, resizable, background `--bg-list`
- Header: title + scope label + count, sort selector; category filters below it.
  Desktop account filtering and search live in the sidebar; mobile keeps them
  above the list.
- Virtualized scroll for performance
- Rows: 72px, flat on the pane — avatar with one unread dot, sender, subject,
  snippet, timestamp, quick actions on hover. The open thread is the only sheet
  in the list (`--bg-surface` + `--shadow-sm`); bundles and stacks are quieter
  than a mail, not louder (`--bg-stack` with a muted border)

### Detail Pane (right)
- Flex-grow to fill remaining space, background `--bg-surface` — the sheet
- Toolbar: `mail/ThreadToolbar.vue` — Done as a primary pill, the rest
  IconButtons; soft pill buttons right
- Thread header: subject at `--font-size-2xl`, then the account dot with its
  name and the message count
- Messages sit flat on the canvas, separated by a rule and their sender header;
  on the desktop the body indents 52px to the sender's name
- Reply opens `ComposerHost`; there is no second composer in the pane

The standalone message viewer (`src/viewer/`) uses the same surface: the body on
`--bg-surface`, the sheet the client reads on. An `.eml` opened from the file
manager is the same mail as the one in the client and should not look like a
different product.

---

## UI Component Library (`src/components/ui/`)

Reusable building blocks — use these instead of hand-rolled buttons/rows:

| Component           | Purpose                                                        |
|---------------------|----------------------------------------------------------------|
| `NavItem`           | Sidebar/tree row: icon or avatar, label, count. Active = a raised sheet (`--bg-surface` + `--shadow-sm`), not an accent pill. Pass `pressed` and the row becomes a toggle that says so. Grows to a 48px touch target below 1020px |
| `IconButton`        | Icon-only button, sizes sm/md/lg, tones default/accent/danger/warning |
| `AppIcon`           | The frame for a stroke icon: 16 viewBox, stroke 1.5, round caps, `aria-hidden`. Sizes sm 14 / md 16 / lg 20; `grid="20"` or `"24"` for a glyph drawn on a larger grid. The glyph is the slot |
| `AppButton`         | Text action, pill variants primary/soft/secondary/ghost/danger and inline variant link, sizes sm/md. `type="submit"` for the button that carries a form's Enter key |
| `CardButton`        | A button that is a *region* — an attachment card, a row, a tile. Contributes the reset and the behaviour, no padding, fill, radius or border: the look stays with the caller |
| `FilterPill`        | One pill in a filter strip: pressed state, optional identity dot and count. `pressed`, not `active` — a strip is a set of toggles |
| `SearchField`       | Search entry in the sidebar or mobile list. A button shaped like a field that opens the shared omni search |
| `CountBadge`        | Rounded count badge, tones neutral/accent/danger               |
| `SectionHeading`    | Group label ("Heute", "Ordner") with optional count/trailing slot |
| `PaneHeader`        | Pane top bar: title, subtitle, count, actions slot, drag region |
| `SegmentedControl`  | Pill-shaped grouped toggle                                     |
| `ModalShell`        | The frame every full modal shares: one scrim, focus trap, focus return, `aria-modal`, no page scroll behind it. Full bleed below the single-pane breakpoint |
| `BottomSheet`       | The small transient surface: rises from the bottom edge on a phone, anchored popover on a desktop. Not a modal, on purpose |
| `ToggleSwitch`      | On/off switch. `before-change` may return `false` to refuse the flip; the knob then stays where it was |
| `EmptyState`, `SkeletonRow`, `ToastStack` | Existing utility components             |

### Adoption, measured

Files outside `src/components/ui/` that import each block, 2026-09-09. The
number is the point of the table: a block with one caller is a suggestion, not
a system, and the two at the bottom are what UI-12 and UI-14 are for.

| 40+ | 20+ | 5–15 | under 5 |
|-----|-----|------|---------|
| `AppButton` 42 | `IconButton` 23 | `ToggleSwitch` 13, `SegmentedControl` 7, `SectionHeading` 7, `ModalShell` 7, `EmptyState` 7, `BottomSheet` 6, `SkeletonRow` 5, `PaneHeader` 5, `CountBadge` 5 | `ToastStack` 3, `NavItem` 3, `ErrorState` 3, `AppBanner` 3, `MenuItem` 2, `ContextMenu` 2, `UndoFab` 1 |

Re-measure it when the number changes, rather than guessing:

```sh
for c in src/components/ui/*.vue; do
  n=$(grep -rl "components/ui/$(basename "$c")" src | grep -vc '^src/components/ui/')
  echo "$n $(basename "$c" .vue)"
done | sort -rn
```

Feature blocks live next to their domain (e.g. `mail/ThreadToolbar.vue` for the
thread-view action bar).

The kit uses itself: `EmptyState` renders its action as an `AppButton`, not as
a copy of one. A block that hand-rolls what another block already does is the
same bug as a feature doing it, one floor down.

One row is deliberately not in the kit: the full-width call-to-action that
opens account setup (`settings/AccountSetup.vue`). It is an icon tile, a label
and a trailing arrow on a tinted, accent-bordered card — neither a pill
(`AppButton`) nor a sidebar row (`NavItem`), and it has exactly one caller. A
kit component with one caller is how the library ended up as a set of
suggestions rather than a system, so this stays a hand-rolled button, named as
such in the check's known list. A second caller turns it into a component.

A dialog does not build its own `position: fixed; inset: 0` scrim. `ModalShell`
carries the scrim, the focus trap, the focus handed back to whatever opened the
dialog, `role="dialog"` with `aria-modal`, and the scroll lock behind it — all
things that were wrong in eleven separate places while the shell had no callers
at all. Escape stays outside it: `ui.dismissTopmostOverlay()` knows the nesting
order, and a second handler on the frame would close the wrong surface. Call
sites pass a `label`, a desktop `width`/`height` and nothing else; below the
single-pane breakpoint the shell ignores both and takes the screen.

---

## Icons

Stroke icons, no icon font and no package — nothing is loaded from outside the
bundle. Every icon is drawn in `AppIcon`:

- **Grid:** 16 × 16. A glyph drawn on 20 or 24 passes `grid`; the frame scales
  it and keeps the line weight.
- **Line:** 1.5, round caps and joins, `fill="none"`, `currentColor`.
- **Sizes:** `sm` 14px (inline with small text, chips), `md` 16px (menus,
  buttons — the default), `lg` 20px (toolbar and header buttons). Nothing in
  between: a 16-grid icon rendered at 13 or 17px loses its crisp edges.
- **Name:** never on the icon. The button around it carries it (`aria-label` or
  visible text); the icon is `aria-hidden`.

The design check freezes the older hand-written SVGs that still break this and
lets the list only shrink. Illustrations and the undo ring are drawings, not
icons, and are listed as such.

---

## Component Patterns

### Buttons
- **Primary:** `AppButton variant="primary"` — accent-primary pill, `--text-on-accent`, hover `--accent-primary-hover`
- **Soft:** `AppButton variant="soft"` — `--bg-raised` pill (e.g. "Antworten", "Weiterleiten")
- **Inline:** `AppButton variant="link"` inherits the surrounding text metrics for actions inside a sentence or identity.
- **Ghost/Tool:** `IconButton` — transparent, hover → `--bg-hover`
- **Danger:** `--accent-red` on hover/active states

### Menus

Every menu row is a `MenuItem`: context menus, the thread toolbar's "More"
menu and its flyouts. **Hover and keyboard focus fill the row with
`--bg-hover` and lift the text to `--text-primary`**; focus adds the ring on
top. A solid `--accent-primary` row is not a menu hover — the blue pill means
*active navigation* (`NavItem`), and a long menu flooding blue under the
pointer reads as a warning. A destructive row takes `tone="danger"`.

The row carries an icon (a named glyph, or the `icon` slot drawn in `AppIcon`
for a glyph only one menu needs), the label, an optional `trailing` slot for
the shortcut, and a chevron when it opens a flyout. The flyout's parent stays
highlighted while the flyout is open.

### Badges / Status Chips
- Small rounded pill: `--radius-xs`
- Colored dot + label: `bg: color/20%, text: color, border: color/30%`
- Workflow states: blue=inbox, amber=todo, cyan=waiting, green=done, purple=snoozed

### Inputs
- Border: `--border-color`, radius: `--radius-sm`
- Background: `--bg-raised`
- Focus: `border-color: --accent-primary`
- Placeholder: `--text-tertiary`

### Scrollbars
- Width: 6px
- Track: transparent
- Thumb: `--border-color`, radius: 3px

### Selection
- `::selection` background: accent-primary at 30% opacity

---

## Error States

A backend failure has exactly three homes. Pick by how long the problem lasts,
not by which file the code happens to sit in. Do not invent a fourth.

| Channel | Use for | Shape |
|---------|---------|-------|
| `layout/ConnectionBanner.vue` | A standing condition — a subsystem is down and stays down until it is retried | `AppBanner tone="danger"`, `role="alert"`, retry action |
| `useToast().error(…)` | A single event that is already over — one send, one delete, one import | `ToastStack` renders `tone === 'error'` with `role="alert"` |
| `.state.state--error` | A list or pane that cannot show its content (`SearchView.vue`) | `<p class="state state--error" role="alert">` plus a retry control |

Rules that hold for all three:

- An error must not look like a success. The neutral grey status line is not an
  error channel — it is what "Draft saved" looks like.
- The headline is a translated cause, not a raw backend message. Subsystems map
  to a static `errors.source.*` key and causes go through `backendErrorLabel()`,
  both in `utils/backendErrorLabels.ts`. A raw message belongs in a collapsed
  `<details>`, the way `OutboxView.vue` shows one.
- A view that is empty *because* a load failed must not render its empty state.
  Read `connection.backendErrorFor(source)` and show the error state instead;
  "no mail" and "we could not fetch your mail" are different sentences.
- Report every backend failure through `connection.reportBackendError(source, err)`
  and clear it with `clearBackendError(source)` on the next success. The banner
  is the only thing a user sees who never opens the failing view.
