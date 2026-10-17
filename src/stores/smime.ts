import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import {
  backendAddSmimeCert,
  backendDeleteSmimeCert,
  backendGenerateSmimeKeypair,
  backendImportSmimePkcs12,
  backendListSmimeCerts,
  backendUnlockSmimeCerts,
  type BackendSmimeCert,
} from '@/services/backend';

export const useSmimeStore = defineStore('smime', () => {
  const certs = ref<BackendSmimeCert[]>([]);
  const loading = ref(false);

  const privateCerts = computed(() => certs.value.filter((cert) => cert.cert_type === 'private'));
  const publicCerts = computed(() => certs.value.filter((cert) => cert.cert_type === 'public'));

  async function load() {
    loading.value = true;
    try {
      certs.value = await backendListSmimeCerts();
    } finally {
      loading.value = false;
    }
  }

  async function addPem(input: { label?: string; pem: string; passphrase?: string }) {
    const cert = await backendAddSmimeCert(input);
    await load();
    return cert;
  }

  async function importPkcs12(input: { label?: string; p12Base64: string; passphrase: string }) {
    const cert = await backendImportSmimePkcs12(input);
    await load();
    return cert;
  }

  async function generate(input: { name: string; email: string; label?: string; passphrase?: string }) {
    const result = await backendGenerateSmimeKeypair(input);
    await load();
    return result;
  }

  async function unlock(certId?: string, passphrase?: string) {
    const result = await backendUnlockSmimeCerts({ certId, passphrase });
    await load();
    return result;
  }

  async function remove(certId: string) {
    await backendDeleteSmimeCert(certId);
    await load();
  }

  function certForEmail(email: string): BackendSmimeCert | undefined {
    const normalized = email.trim().toLowerCase();
    return certs.value.find((cert) => cert.subject_email?.toLowerCase() === normalized);
  }

  function privateCertForEmail(email: string): BackendSmimeCert | undefined {
    const normalized = email.trim().toLowerCase();
    return privateCerts.value.find((cert) => cert.subject_email?.toLowerCase() === normalized);
  }

  return {
    certs,
    loading,
    privateCerts,
    publicCerts,
    load,
    addPem,
    importPkcs12,
    generate,
    unlock,
    remove,
    certForEmail,
    privateCertForEmail,
  };
});
