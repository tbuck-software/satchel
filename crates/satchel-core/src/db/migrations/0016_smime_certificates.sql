CREATE TABLE IF NOT EXISTS smime_certificates (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    cert_type TEXT NOT NULL CHECK (cert_type IN ('private', 'public')),
    fingerprint TEXT NOT NULL,
    subject_email TEXT,
    subject_cn TEXT,
    issuer_cn TEXT,
    not_before TEXT,
    not_after TEXT,
    pem_certificate TEXT NOT NULL,
    pem_private_key_encrypted TEXT,
    has_passphrase INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_smime_cert_fingerprint
  ON smime_certificates(fingerprint, cert_type);

CREATE INDEX IF NOT EXISTS idx_smime_cert_email
  ON smime_certificates(LOWER(subject_email));

ALTER TABLE messages ADD COLUMN smime_encrypted BLOB NULL;
ALTER TABLE messages ADD COLUMN smime_signed BLOB NULL;
