import { shallowRef, type Component } from 'vue';

/**
 * Rows a switched-on Labs experiment adds under the Inbox in the sidebar.
 * This is all of Labs that startup code holds: the flags, the store and each
 * experiment's code load behind `import('@/labs/runtime')`, and an experiment
 * puts its row here when it activates (`src/labs/activation.ts`).
 */
export const labsSidebarItems = shallowRef<Component[]>([]);
