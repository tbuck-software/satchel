/**
 * Glyphs for the command palette, drawn for `AppIcon`. Each is the one the
 * same action already wears elsewhere — the toolbar, its menu, the sidebar —
 * so a command looks like the button it stands in for. Circles are written as
 * two arcs because the frame takes paths only.
 */
export interface CommandGlyph {
  grid: 16 | 20 | 24;
  paths: readonly string[];
}

const circle = (cx: number, cy: number, r: number) =>
  `M${cx + r} ${cy}a${r} ${r} 0 1 1-${2 * r} 0a${r} ${r} 0 1 1 ${2 * r} 0`;

export const COMMAND_ICONS = {
  // Thread toolbar menu, 20 grid.
  reply: { grid: 20, paths: ['M8 5 3 10l5 5M3 10h8a5 5 0 0 1 5 5'] },
  replyAll: { grid: 20, paths: ['M7 5 2 10l5 5', 'M11 5 6 10l5 5M6 10h6a5 5 0 0 1 5 5'] },
  forward: { grid: 20, paths: ['m12 5 5 5-5 5M17 10H9a5 5 0 0 0-5 5'] },
  envelope: { grid: 20, paths: ['M5 5h10a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2Z', 'm4 7 6 4 6-4'] },
  pin: { grid: 20, paths: ['M6.5 3h7l-1 5 2.5 2.5V12H5v-1.5L7.5 8l-1-5Z', 'M10 12v5'] },
  link: {
    grid: 20,
    paths: [
      'M8.5 11.5a3.2 3.2 0 0 0 4.6 0l2.6-2.6a3.2 3.2 0 0 0-4.6-4.6l-1.1 1.1',
      'M11.5 8.5a3.2 3.2 0 0 0-4.6 0l-2.6 2.6a3.2 3.2 0 0 0 4.6 4.6l1.1-1.1',
    ],
  },
  mute: { grid: 20, paths: ['M5 8v4h3l4 3V5L8 8H5Z', 'm15 8 3 4m0-4-3 4'] },
  print: { grid: 20, paths: ['M6 7V3h8v4M5 15H3V8h14v7h-2M6 12h8v5H6v-5Z'] },
  export: { grid: 20, paths: ['M10 3v10m0 0-4-4m4 4 4-4', 'M4 15v2h12v-2'] },
  account: { grid: 20, paths: [circle(10, 7, 3), 'M4 17a6 6 0 0 1 12 0'] },
  // Sidebar and mailbox tree, 16 grid.
  done: { grid: 16, paths: ['M3 8.5l3.3 3.3L13 5'] },
  clock: { grid: 16, paths: [circle(8, 8, 5.5), 'M8 5.25v3.2L10.4 9.8'] },
  aside: { grid: 16, paths: ['M2.5 5.5h11v7.5h-11z', 'M5.5 8.5h5', 'M8 1.8v4', 'M6.2 4L8 5.8 9.8 4'] },
  inbox: { grid: 16, paths: ['M2 5.5h12', 'M3 5.5v6.5A1.5 1.5 0 004.5 13.5h7A1.5 1.5 0 0013 12V5.5', 'M5.5 8.5h5'] },
  trash: { grid: 16, paths: ['M2.5 4h11', 'M6 2.5h4', 'M4 4l.6 9a1 1 0 001 1h4.8a1 1 0 001-1L12 4'] },
  spam: { grid: 16, paths: ['M8 1.5l5 2.9v7.2L8 14.5l-5-2.9V4.4L8 1.5Z', 'M8 5.1v3.5', 'M8 11h.01'] },
  block: { grid: 16, paths: [circle(8, 8, 5.5), 'M4.5 11.5l7-7'] },
  folder: { grid: 16, paths: ['M2 4.5h4l1.2 1.5H14v6.5a1 1 0 01-1 1H3a1 1 0 01-1-1z'] },
  topic: {
    grid: 16,
    paths: ['M2 4.5C2 3.7 2.7 3 3.5 3H6L7.5 4.5H12.5C13.3 4.5 14 5.2 14 6V11.5C14 12.3 13.3 13 12.5 13H3.5C2.7 13 2 12.3 2 11.5V4.5Z'],
  },
  compose: { grid: 16, paths: ['M12 1.5L14.5 4L5.5 13H3V10.5L12 1.5Z', 'M10 3.5L12.5 6'] },
  calendar: { grid: 16, paths: ['M3.5 2.5h9a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2Z', 'M1.5 6h13M5 1v2.5M11 1v2.5'] },
  waiting: { grid: 16, paths: ['M4.5 2.5h7M4.5 13.5h7', 'M5.5 2.5v2L8 8l2.5-3.5v-2', 'M5.5 13.5v-2L8 8l2.5 3.5v2'] },
  undo: { grid: 16, paths: ['M3.5 6.5h6a3.2 3.2 0 010 6.4H6', 'M6 3.5L3 6.5l3 3'] },
  sync: { grid: 16, paths: ['M13.5 6A5.7 5.7 0 0 0 3.7 3.7L1.5 6m0-4v4h4M2.5 10a5.7 5.7 0 0 0 9.8 2.3l2.2-2.3m0 4v-4h-4'] },
  theme: { grid: 16, paths: ['M13.5 9.6A5.5 5.5 0 1 1 6.4 2.5a4.5 4.5 0 0 0 7.1 7.1Z'] },
  keyboard: {
    grid: 16,
    paths: [
      'M2.5 4h11a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Z',
      'M4.5 6.5h.01M7 6.5h.01M9 6.5h.01M11.5 6.5h.01M5 9.5h6',
    ],
  },
  // Sidebar settings gear, 24 grid.
  settings: {
    grid: 24,
    paths: [
      'M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z',
      circle(12, 12, 3),
    ],
  },
} as const satisfies Record<string, CommandGlyph>;

export type CommandIconName = keyof typeof COMMAND_ICONS;
