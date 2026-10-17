//! Scrubs text before it leaves the machine in a copied diagnostics report.
//!
//! Removed: email addresses, credentials and opaque tokens, URL query strings,
//! hostnames and IPv4 addresses other than well-known public mail providers,
//! the user name in home paths, and mail content behind content keys.

use std::sync::LazyLock;

use regex::{Captures, Regex};

const REDACTED: &str = "<redacted>";

/// Hosts that say which provider is involved without naming anyone's server.
const PUBLIC_DOMAINS: &[&str] = &[
    "aol.com",
    "apple.com",
    "example.com",
    "example.org",
    "fastmail.com",
    "github.com",
    "gmail.com",
    "gmx.com",
    "gmx.de",
    "gmx.net",
    "google.com",
    "googleapis.com",
    "googlemail.com",
    "googleusercontent.com",
    "hotmail.com",
    "icloud.com",
    "live.com",
    "mac.com",
    "mailbox.org",
    "me.com",
    "microsoft.com",
    "microsoftonline.com",
    "office.com",
    "office365.com",
    "outlook.com",
    "posteo.de",
    "proton.me",
    "protonmail.com",
    "t-online.de",
    "web.de",
    "yahoo.com",
    "yandex.com",
    "zoho.com",
];

/// Dotted names that are code, not hosts: our bundle id, JVM frames in
/// Android exit reports.
const CODE_PREFIXES: &[&str] = &[
    "android.",
    "androidx.",
    "com.android.",
    "com.google.android.",
    "dalvik.",
    "de.tbuck.",
    "java.",
    "javax.",
    "jdk.",
    "kotlin.",
    "kotlinx.",
    "libcore.",
    "sun.",
];

const FILE_EXTENSIONS: &[&str] = &[
    "c", "cjs", "conf", "cpp", "css", "db", "dylib", "eml", "gz", "h", "html", "ics", "java",
    "jpeg", "jpg", "js", "json", "kt", "lock", "log", "mbox", "mjs", "msg", "pdf", "plist", "png",
    "shm", "sqlite", "svg", "swift", "tmp", "toml", "ts", "tsx", "txt", "vcf", "vue", "wal",
    "wasm", "webp", "yaml", "yml", "zip",
];

/// Extensions that are country TLDs too: only `name.ext` counts as a file.
const AMBIGUOUS_EXTENSIONS: &[&str] = &["md", "mm", "py", "rs", "sh", "so"];

/// Every two-letter label counts as a country TLD. Without a TLD check,
/// i18n keys (`backend.errors.x`) and JS members (`threads.find`) read as hosts.
const GENERIC_TLDS: &[&str] = &[
    "agency",
    "app",
    "arpa",
    "bayern",
    "berlin",
    "biz",
    "blog",
    "cloud",
    "club",
    "com",
    "company",
    "consulting",
    "corp",
    "dev",
    "digital",
    "edu",
    "email",
    "family",
    "gmbh",
    "gov",
    "group",
    "hamburg",
    "home",
    "host",
    "info",
    "int",
    "internal",
    "intranet",
    "koeln",
    "lan",
    "link",
    "live",
    "local",
    "localdomain",
    "mail",
    "net",
    "network",
    "nrw",
    "online",
    "org",
    "page",
    "pro",
    "ruhr",
    "server",
    "services",
    "shop",
    "site",
    "solutions",
    "space",
    "store",
    "systems",
    "team",
    "tech",
    "tools",
    "website",
    "wien",
    "work",
    "works",
    "world",
    "xyz",
    "zone",
];

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static redaction pattern")
}

static CONTENT_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?i)\b(subject|snippet|body|preview|from_name|display_name|sender_name|filename|file_name|query)("?\s*[=:]\s*)("(?:[^"\\]|\\.)*"|[^\s,;}\]]+)"#,
    )
});
static SECRET_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r#"(?i)\b(password|passwd|pwd|secret|client_secret|token|access_token|refresh_token|id_token|auth_code|code_verifier|authorization|api_key|apikey|cookie)("?\s*[=:]\s*"?)(?:(?:bearer|basic)\s+)?[^\s"',;&}]+"#,
    )
});
static AUTH_SCHEME: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}"));
static JWT: LazyLock<Regex> =
    LazyLock::new(|| regex(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*"));
static URL_USERINFO: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/@"'<>]+@"#));
static URL_QUERY: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"(?i)\b([a-z][a-z0-9+.-]*://[^\s?#"'<>]*)\?[^\s"'<>]*"#));
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| regex(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"));
static HOME_PATH: LazyLock<Regex> =
    LazyLock::new(|| regex(r"(/Users/|/home/|(?i:\b[a-z]:\\Users\\))[^/\\\s]+"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| regex(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"));
static HOSTNAME: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"(?i)\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]{0,22}[a-z]\b")
});
static OPAQUE: LazyLock<Regex> = LazyLock::new(|| regex(r"[A-Za-z0-9+/_=-]{32,}"));

/// Applies every rule. Order matters: content and secrets first, so a quoted
/// subject is gone before the host and email rules see pieces of it.
pub fn redact(text: &str) -> String {
    let text = CONTENT_FIELD.replace_all(text, format!("${{1}}${{2}}{REDACTED}"));
    let text = SECRET_FIELD.replace_all(&text, format!("${{1}}${{2}}{REDACTED}"));
    let text = AUTH_SCHEME.replace_all(&text, format!("${{1}} {REDACTED}"));
    let text = JWT.replace_all(&text, "<token>");
    let text = URL_USERINFO.replace_all(&text, format!("${{1}}{REDACTED}@"));
    let text = URL_QUERY.replace_all(&text, format!("${{1}}?{REDACTED}"));
    let text = EMAIL.replace_all(&text, "<email>");
    let text = HOME_PATH.replace_all(&text, "${1}<user>");
    let text = IPV4.replace_all(&text, |caps: &Captures| {
        let ip = &caps[0];
        if ip == "127.0.0.1" || ip == "0.0.0.0" {
            ip.to_string()
        } else {
            "<ip>".to_string()
        }
    });
    let text = HOSTNAME.replace_all(&text, |caps: &Captures| {
        let name = &caps[0];
        if keeps_hostname(name) {
            name.to_string()
        } else {
            "<host>".to_string()
        }
    });
    let text = OPAQUE.replace_all(&text, |caps: &Captures| {
        let run = &caps[0];
        if looks_like_token(run) {
            "<token>".to_string()
        } else {
            run.to_string()
        }
    });
    text.into_owned()
}

fn keeps_hostname(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let labels = lower.split('.').count();
    let extension = lower.rsplit('.').next().unwrap_or_default();
    if FILE_EXTENSIONS.contains(&extension) {
        return true;
    }
    if AMBIGUOUS_EXTENSIONS.contains(&extension) && labels == 2 {
        return true;
    }
    if CODE_PREFIXES.iter().any(|prefix| lower.starts_with(prefix)) {
        return true;
    }
    if extension.len() != 2 && !GENERIC_TLDS.contains(&extension) {
        return true;
    }
    lower == "localhost"
        || PUBLIC_DOMAINS
            .iter()
            .any(|domain| lower == *domain || lower.ends_with(&format!(".{domain}")))
}

/// Mixed case and digits in one long run, and not a path.
fn looks_like_token(run: &str) -> bool {
    run.bytes().any(|b| b.is_ascii_digit())
        && run.bytes().any(|b| b.is_ascii_uppercase())
        && run.bytes().any(|b| b.is_ascii_lowercase())
        && run.matches('/').count() < 2
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn removes_email_addresses() {
        let out = redact("sync failed for jane.doe+work@firma-intern.de: AUTHENTICATIONFAILED");
        assert_eq!(out, "sync failed for <email>: AUTHENTICATIONFAILED");
    }

    #[test]
    fn removes_credentials_and_tokens() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.c2lnbmF0dXJlLXNpZ24";
        let cases = [
            ("C: A1 LOGIN user password=hunter2", "hunter2"),
            ("Authorization: Bearer ya29.a0AfH6SMBx-secret", "ya29"),
            ("refresh_token=1//0gAbCdEfGhIjKlMnOpQrStUv", "0gAbCd"),
            ("{\"access_token\":\"abc.def.ghi\"}", "abc.def"),
            (
                "token rejected: ya29A0AfH6SMBxQ9vZt7kLmNoPqRsTuVwXyZ012345",
                "ya29A0",
            ),
        ];
        for (input, secret) in cases {
            let out = redact(input);
            assert!(!out.contains(secret), "{input} -> {out}");
        }
        assert_eq!(redact(&format!("id {jwt} expired")), "id <token> expired");
    }

    #[test]
    fn removes_private_hosts_and_addresses_but_keeps_public_providers() {
        let out = redact("connect imap.mail.firma-intern.de:993 (10.0.4.17) via imap.gmail.com");
        assert_eq!(out, "connect <host>:993 (<ip>) via imap.gmail.com");
        assert_eq!(redact("nas.local refused"), "<host> refused");
        assert_eq!(
            redact("outlook.office365.com and 127.0.0.1 are fine"),
            "outlook.office365.com and 127.0.0.1 are fine"
        );
    }

    #[test]
    fn removes_url_userinfo_and_query_strings() {
        let out = redact("GET https://dav.example-family.net/cal?user=me&code=xyz failed");
        assert_eq!(out, "GET https://<host>/cal?<redacted> failed");
        let out = redact("imaps://jane:secret@mail.privat.de");
        assert!(!out.contains("secret") && !out.contains("jane"), "{out}");
    }

    #[test]
    fn removes_message_content_fields() {
        let out = redact(
            r#"WARN parse failed subject="Kündigung zum 31.12." snippet=Hallo from_name="Jane Doe""#,
        );
        assert!(!out.contains("Kündigung"), "{out}");
        assert!(!out.contains("Hallo"), "{out}");
        assert!(!out.contains("Jane"), "{out}");
        assert!(out.contains("subject=<redacted>"), "{out}");
    }

    #[test]
    fn removes_the_user_name_from_home_paths() {
        assert_eq!(
            redact("open /Users/jane/Library/Logs/x failed"),
            "open /Users/<user>/Library/Logs/x failed"
        );
        assert_eq!(redact(r"C:\Users\Jane\AppData"), r"C:\Users\<user>\AppData");
    }

    #[test]
    fn keeps_what_a_bug_report_needs() {
        let line = "2026-10-13T08:00:00.1Z  WARN satchel::sync::mailbox_sync: \
                    src/sync/mailbox_sync.rs:412: retry account=3f2c9a1e-8d5b-4c7e-9f00-1a2b3c4d5e6f \
                    version=0.1.0 file=tauri.conf.json frame=android.os.Looper.loop(Looper.java:123) \
                    bundle=de.tbuck.satchel commit=abc123def456 \
                    error={\"key\":\"backend.errors.accountSyncFailed\"} \
                    js=\"threads.find is not a function\"";
        assert_eq!(redact(line), line);
    }
}
