//! OpenSSL chain building for platforms without a native S/MIME verifier.
use openssl::{
    stack::Stack,
    x509::{
        store::{X509Lookup, X509Store, X509StoreBuilder},
        X509PurposeId, X509StoreContext, X509,
    },
};
use std::path::Path;

pub(in crate::utils) fn verify(leaf: &[u8], certificates: &[Vec<u8>]) -> Result<(), String> {
    let mut builder = X509StoreBuilder::new().map_err(|e| e.to_string())?;
    load_system_roots(&mut builder)?;
    builder
        .set_purpose(X509PurposeId::SMIME_SIGN)
        .map_err(|e| e.to_string())?;
    verify_with_store(leaf, certificates, &builder.build())
}

pub(crate) fn verify_with_store(
    leaf: &[u8],
    certificates: &[Vec<u8>],
    store: &X509Store,
) -> Result<(), String> {
    let leaf = X509::from_der(leaf).map_err(|e| e.to_string())?;
    let mut candidates = Stack::new().map_err(|e| e.to_string())?;
    for der in certificates {
        candidates
            .push(X509::from_der(der).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    }
    let mut context = X509StoreContext::new().map_err(|e| e.to_string())?;
    context
        .init(store, &leaf, &candidates, |ctx| {
            if ctx.verify_cert()? {
                Ok(Ok(()))
            } else {
                Ok(Err(ctx.error().to_string()))
            }
        })
        .map_err(|e| e.to_string())?
}

fn load_system_roots(builder: &mut X509StoreBuilder) -> Result<(), String> {
    // Honor OpenSSL trust/reject attributes in distribution bundles. Prefer
    // purpose-aware bundles so TLS-only trust cannot become S/MIME trust.
    let policy_bundle = [
        "/etc/pki/ca-trust/extracted/openssl/ca-bundle.trust.crt",
        "/etc/pki/ca-trust/extracted/pem/email-ca-bundle.pem",
    ]
    .into_iter()
    .find(|path| Path::new(path).is_file());
    let explicit =
        std::env::var_os("SSL_CERT_FILE").is_some() || std::env::var_os("SSL_CERT_DIR").is_some();
    if explicit || policy_bundle.is_none() {
        builder.set_default_paths().map_err(|e| e.to_string())?;
    }
    if !explicit {
        let bundle = policy_bundle.or_else(|| {
            [
                "/etc/ssl/cert.pem",
                "/etc/ssl/certs/ca-certificates.crt",
                "/etc/pki/tls/certs/ca-bundle.crt",
                "/opt/homebrew/etc/openssl@3/cert.pem",
                "/usr/local/etc/openssl@3/cert.pem",
            ]
            .into_iter()
            .find(|path| Path::new(path).is_file())
        });
        if let Some(path) = bundle {
            builder
                .add_lookup(X509Lookup::file())
                .map_err(|e| e.to_string())?
                .load_cert_file(path, openssl::ssl::SslFiletype::PEM)
                .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(target_os = "android")]
    {
        for dir in [
            "/system/etc/security/cacerts",
            "/apex/com.android.conscrypt/cacerts",
            "/data/misc/user/0/cacerts-added",
        ] {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if let Ok(pem) = std::fs::read(entry.path()) {
                    if let Ok(certs) = X509::stack_from_pem(&pem) {
                        for cert in certs {
                            builder.add_cert(cert).map_err(|e| e.to_string())?;
                        }
                    }
                }
            }
        }
        for der in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
            builder
                .add_cert(X509::from_der(der).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
