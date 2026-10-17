import type { SmimeCertificateDto as WireSmimeCertificateDto } from './generated/app/commands/smime/SmimeCertificateDto';
import type { OptionalWireFields } from './wireTypes';
import type { PgpKeyDto as WirePgpKeyDto } from './generated/app/commands/pgp/PgpKeyDto';
import type { ImportPgpKeysResult as WireImportPgpKeysResult } from './generated/app/commands/pgp/ImportPgpKeysResult';
import type { GeneratePgpKeyResult as WireGeneratePgpKeyResult } from './generated/app/commands/pgp/GeneratePgpKeyResult';
import type { UnlockPgpKeysResult as WireUnlockPgpKeysResult } from './generated/app/commands/pgp/UnlockPgpKeysResult';
import type { ValidatePgpBundlePassphraseResult as WireValidatePgpBundlePassphraseResult } from './generated/app/commands/pgp/ValidatePgpBundlePassphraseResult';
import type { GenerateSmimeResult as WireGenerateSmimeResult } from './generated/app/commands/smime/GenerateSmimeResult';
import type { UnlockSmimeResult as WireUnlockSmimeResult } from './generated/app/commands/smime/UnlockSmimeResult';
import type { PgpEngineStatus as WirePgpEngineStatus } from './generated/app/commands/pgp/PgpEngineStatus';
// Auto-split from services/backend.ts — domain module.
import { invoke } from './core';

export type BackendPgpKey = WirePgpKeyDto;


export type BackendImportPgpKeysResult = WireImportPgpKeysResult;


export type BackendGeneratePgpKeyResult = WireGeneratePgpKeyResult;


export type BackendUnlockPgpKeysResult = WireUnlockPgpKeysResult;


export type BackendValidatePgpBundlePassphraseResult = WireValidatePgpBundlePassphraseResult;


export type BackendSmimeCert = OptionalWireFields<WireSmimeCertificateDto, 'subject_email' | 'subject_cn' | 'issuer_cn' | 'not_before' | 'not_after'>;


export type BackendGenerateSmimeResult = WireGenerateSmimeResult;


export type BackendUnlockSmimeResult = WireUnlockSmimeResult;


export type BackendPgpEngineStatus = WirePgpEngineStatus;


export async function backendPgpEngineStatus(): Promise<BackendPgpEngineStatus> {
  return invoke<BackendPgpEngineStatus>('pgp_engine_status');
}


export async function backendListPgpKeys(): Promise<BackendPgpKey[]> {
  return invoke<BackendPgpKey[]>('list_pgp_keys');
}


export async function backendImportPgpKeys(payload: {
  armoredBundle: string;
  labelPrefix?: string;
  passphrase?: string;
}): Promise<BackendImportPgpKeysResult> {
  return invoke<BackendImportPgpKeysResult>('import_pgp_keys', {
    input: {
      armored_bundle: payload.armoredBundle,
      label_prefix: payload.labelPrefix?.trim() || null,
      passphrase: payload.passphrase?.trim() || null,
    },
  });
}


export async function backendValidatePgpBundlePassphrase(payload: {
  armoredBundle: string;
  passphrase: string;
}): Promise<BackendValidatePgpBundlePassphraseResult> {
  return invoke<BackendValidatePgpBundlePassphraseResult>('validate_pgp_bundle_passphrase', {
    input: {
      armored_bundle: payload.armoredBundle,
      passphrase: payload.passphrase,
    },
  });
}


export async function backendGeneratePgpKey(payload: {
  name: string;
  email: string;
  label?: string;
  passphrase?: string;
  algorithm?: string;
  expiresIn?: string;
}): Promise<BackendGeneratePgpKeyResult> {
  return invoke<BackendGeneratePgpKeyResult>('generate_pgp_key', {
    input: {
      name: payload.name,
      email: payload.email,
      label: payload.label?.trim() || null,
      passphrase: payload.passphrase?.trim() || null,
      algorithm: payload.algorithm?.trim() || null,
      expires_in: payload.expiresIn?.trim() || null,
    },
  });
}


export async function backendUnlockPgpKeys(payload?: {
  keyId?: string;
  passphrase?: string;
}): Promise<BackendUnlockPgpKeysResult> {
  return invoke<BackendUnlockPgpKeysResult>('unlock_pgp_keys', {
    input: {
      key_id: payload?.keyId?.trim() || null,
      passphrase: payload?.passphrase?.trim() || null,
    },
  });
}


export async function backendDeletePgpKey(keyId: string): Promise<void> {
  return invoke<void>('delete_pgp_key', { keyId });
}


export async function backendListSmimeCerts(): Promise<BackendSmimeCert[]> {
  return invoke<BackendSmimeCert[]>('list_smime_certificates');
}


export async function backendAddSmimeCert(payload: {
  label?: string;
  pem: string;
  passphrase?: string;
}): Promise<BackendSmimeCert> {
  return invoke<BackendSmimeCert>('add_smime_certificate', {
    input: {
      label: payload.label?.trim() || null,
      pem: payload.pem,
      passphrase: payload.passphrase?.trim() || null,
    },
  });
}


export async function backendImportSmimePkcs12(payload: {
  label?: string;
  p12Base64: string;
  passphrase: string;
}): Promise<BackendSmimeCert> {
  return invoke<BackendSmimeCert>('import_smime_pkcs12', {
    input: {
      label: payload.label?.trim() || null,
      p12_base64: payload.p12Base64,
      passphrase: payload.passphrase,
    },
  });
}


export async function backendGenerateSmimeKeypair(payload: {
  name: string;
  email: string;
  label?: string;
  passphrase?: string;
}): Promise<BackendGenerateSmimeResult> {
  return invoke<BackendGenerateSmimeResult>('generate_smime_keypair', {
    input: {
      name: payload.name,
      email: payload.email,
      label: payload.label?.trim() || null,
      passphrase: payload.passphrase?.trim() || null,
    },
  });
}


export async function backendUnlockSmimeCerts(payload?: {
  certId?: string;
  passphrase?: string;
}): Promise<BackendUnlockSmimeResult> {
  return invoke<BackendUnlockSmimeResult>('unlock_smime_certificates', {
    input: {
      cert_id: payload?.certId?.trim() || null,
      passphrase: payload?.passphrase?.trim() || null,
    },
  });
}


export async function backendDeleteSmimeCert(certId: string): Promise<void> {
  return invoke<void>('delete_smime_certificate', { certId });
}
