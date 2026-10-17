import { computed, ref } from 'vue';
import { defineStore } from 'pinia';
import type { WorkflowTransition } from '@/types/workflow';
import { backendListRecentTransitions } from '@/services/backend';

export const useWorkflowStore = defineStore('workflow', () => {
  const transitions = ref<WorkflowTransition[]>([]);

  const recentTransitions = computed(() => transitions.value.slice(-20).reverse());

  function trackTransition(transition: WorkflowTransition) {
    transitions.value.push(transition);
  }

  async function load() {
    try {
      const rows = await backendListRecentTransitions();
      transitions.value = rows.map((row) => ({
        from: row.from,
        to: row.to,
        reason: 'manual',
        at: row.at,
      }));
    } catch {
      // Web mode and local mock mode intentionally fall back silently.
    }
  }

  return {
    transitions,
    recentTransitions,
    trackTransition,
    load,
  };
});
