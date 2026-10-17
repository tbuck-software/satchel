use core_foundation::{
    base::{CFTypeRef, TCFType},
    dictionary::CFDictionaryRef,
    string::CFStringRef,
};
use security_framework::{certificate::SecCertificate, policy::SecPolicy, trust::SecTrust};
use security_framework_sys::base::SecPolicyRef;

// Public Security.framework APIs not yet exposed by security-framework-sys.
// Unlike an SSL or basic X.509 policy, this enforces the S/MIME certificate purpose.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecPolicyAppleSMIME: CFStringRef;
    fn SecPolicyCreateWithProperties(
        identifier: CFTypeRef,
        properties: CFDictionaryRef,
    ) -> SecPolicyRef;
}

pub(super) fn create_trust(leaf: &[u8], certificates: &[Vec<u8>]) -> Result<SecTrust, String> {
    let mut chain = vec![SecCertificate::from_der(leaf).map_err(|e| e.to_string())?];
    for der in certificates.iter().filter(|der| der.as_slice() != leaf) {
        chain.push(SecCertificate::from_der(der).map_err(|e| e.to_string())?);
    }
    // SAFETY: the identifier is an immutable framework constant; null properties
    // select the default S/MIME policy. The Create result is owned and null-checked.
    let raw =
        unsafe { SecPolicyCreateWithProperties(kSecPolicyAppleSMIME.cast(), std::ptr::null()) };
    if raw.is_null() {
        return Err("Unable to create the native S/MIME trust policy".into());
    }
    let policy = unsafe { SecPolicy::wrap_under_create_rule(raw) };
    let mut trust =
        SecTrust::create_with_certificates(&chain, &[policy]).map_err(|e| e.to_string())?;
    trust
        .set_network_fetch_allowed(false)
        .map_err(|e| e.to_string())?;
    Ok(trust)
}

pub(super) fn verify(leaf: &[u8], certificates: &[Vec<u8>]) -> Result<(), String> {
    // Message viewing must not disclose a read through AIA, OCSP or CRL requests.
    let trust = create_trust(leaf, certificates)?;
    trust.evaluate_with_error().map_err(|e| e.to_string())
}
