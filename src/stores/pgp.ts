import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendListPgpKeys,
  backendPgpEngineStatus,
  backendCommandsAvailable,
  type BackendPgpEngineStatus,
  type BackendPgpKey,
} from '@/services/backend';

export const usePgpStore = defineStore('pgp', () => {
  const keys = ref<BackendPgpKey[]>([]);
  const engine = ref<BackendPgpEngineStatus | null>(null);
  const loading = ref(false);

  const privateKeys = computed(() => keys.value.filter((key) => key.key_type === 'private'));
  const publicKeys = computed(() => keys.value.filter((key) => key.key_type === 'public'));

  /**
   * Signing needs a key that is unlocked for this session — Satchel never
   * stores a usable passphrase, so a locked key cannot sign.
   */
  const unlockedPrivateKeys = computed(() => privateKeys.value.filter((key) => key.is_unlocked));

  /** PGP is offered when the selected in-process or optional CLI engine works. */
  const available = computed(() => engine.value?.available === true);

  async function load() {
    if (!backendCommandsAvailable()) return;
    loading.value = true;
    try {
      const [loadedKeys, status] = await Promise.all([
        backendListPgpKeys().catch(() => [] as BackendPgpKey[]),
        backendPgpEngineStatus().catch(() => null),
      ]);
      keys.value = loadedKeys;
      engine.value = status;
    } finally {
      loading.value = false;
    }
  }

  /** Public key present for this recipient, matched via the key's user IDs. */
  function hasKeyFor(email: string): boolean {
    const needle = asciiLowercase(email.trim());
    if (!needle) return false;
    return keys.value.some((key) =>
      key.can_encrypt && !key.is_expired && key.user_ids.some((userId) => {
        const normalized = asciiLowercase(userId.trim());
        return normalized === needle || normalized.includes(`<${needle}>`);
      }),
    );
  }

  return {
    keys,
    engine,
    loading,
    privateKeys,
    publicKeys,
    unlockedPrivateKeys,
    available,
    load,
    hasKeyFor,
  };
});

function asciiLowercase(value: string): string {
  return value.replace(/[A-Z]/g, (character) => character.toLowerCase());
}
