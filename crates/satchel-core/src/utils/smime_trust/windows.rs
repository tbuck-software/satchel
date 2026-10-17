use std::{mem::size_of, ptr};
use windows_sys::Win32::Security::Cryptography::*;

struct Store(HCERTSTORE);
impl Drop for Store {
    fn drop(&mut self) {
        unsafe {
            CertCloseStore(self.0, 0);
        }
    }
}
struct Certificate(*mut CERT_CONTEXT);
impl Drop for Certificate {
    fn drop(&mut self) {
        unsafe {
            CertFreeCertificateContext(self.0);
        }
    }
}
struct Chain(*mut CERT_CHAIN_CONTEXT);
impl Drop for Chain {
    fn drop(&mut self) {
        unsafe {
            CertFreeCertificateChain(self.0);
        }
    }
}

fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

pub(super) fn verify(leaf: &[u8], certificates: &[Vec<u8>]) -> Result<(), String> {
    // SAFETY: DER buffers remain alive during the copying calls. Every returned
    // handle is null-checked and owned by a guard until chain evaluation ends.
    unsafe {
        let raw = CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, ptr::null());
        if raw.is_null() {
            return Err(last_error());
        }
        let store = Store(raw);
        for der in certificates {
            if CertAddEncodedCertificateToStore(
                store.0,
                X509_ASN_ENCODING,
                der.as_ptr(),
                u32::try_from(der.len()).map_err(|e| e.to_string())?,
                CERT_STORE_ADD_ALWAYS,
                ptr::null_mut(),
            ) == 0
            {
                return Err(last_error());
            }
        }
        let raw = CertCreateCertificateContext(
            X509_ASN_ENCODING,
            leaf.as_ptr(),
            u32::try_from(leaf.len()).map_err(|e| e.to_string())?,
        );
        if raw.is_null() {
            return Err(last_error());
        }
        let leaf = Certificate(raw);
        let mut usage = 0u8;
        if CertGetIntendedKeyUsage(X509_ASN_ENCODING, (*leaf.0).pCertInfo, &mut usage, 1) == 0 {
            return Err(last_error());
        }
        if u32::from(usage) & (CERT_DIGITAL_SIGNATURE_KEY_USAGE | CERT_NON_REPUDIATION_KEY_USAGE)
            == 0
        {
            return Err("Certificate key usage does not permit signing".into());
        }
        let mut email_usage = szOID_PKIX_KP_EMAIL_PROTECTION.cast_mut();
        let parameters = CERT_CHAIN_PARA {
            cbSize: size_of::<CERT_CHAIN_PARA>() as u32,
            RequestedUsage: CERT_USAGE_MATCH {
                dwType: USAGE_MATCH_TYPE_AND,
                Usage: CTL_USAGE {
                    cUsageIdentifier: 1,
                    rgpszUsageIdentifier: &mut email_usage,
                },
            },

            ..Default::default()
        };
        let mut raw = ptr::null_mut();
        // The additional memory store supplies candidates, not trusted roots.
        // The current-user chain engine respects Windows trust and distrust;
        // use only cached issuers, revocation data and root updates.
        if CertGetCertificateChain(
            ptr::null_mut(),
            leaf.0,
            ptr::null(),
            store.0,
            &parameters,
            CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL
                | CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE
                | CERT_CHAIN_DISABLE_AIA
                | CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY,
            ptr::null(),
            &mut raw,
        ) == 0
        {
            return Err(last_error());
        }
        let chain = Chain(raw);
        let policy = CERT_CHAIN_POLICY_PARA {
            cbSize: size_of::<CERT_CHAIN_POLICY_PARA>() as u32,
            ..Default::default()
        };
        let mut status = CERT_CHAIN_POLICY_STATUS {
            cbSize: size_of::<CERT_CHAIN_POLICY_STATUS>() as u32,
            ..Default::default()
        };
        if CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_BASE, chain.0, &policy, &mut status)
            == 0
        {
            return Err(last_error());
        }
        if status.dwError != 0 {
            return Err(format!(
                "Windows S/MIME certificate validation failed: 0x{:08X}",
                status.dwError
            ));
        }
    }
    Ok(())
}
