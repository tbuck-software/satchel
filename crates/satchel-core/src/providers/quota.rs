//! Mailbox quota: how full the server-side mailbox is.
//!
//! IMAP reports it through the QUOTA extension (RFC 2087, revised by
//! RFC 9208). `async-imap` already parses the `QUOTAROOT` and `QUOTA`
//! responses; this module turns them into one reading and leaves the wire to
//! the parser. Gmail and Microsoft Graph only expose a mailbox quota through
//! APIs whose scopes Satchel does not request (Drive, Reports), and POP3 has
//! none, so those providers report [`QuotaProbe::Unsupported`].

use async_imap::types::{Quota, QuotaResourceName, QuotaRoot};
use tokio::time::{timeout, Duration};

/// IMAP's STORAGE resource counts units of 1024 octets.
const STORAGE_UNIT_BYTES: u64 = 1024;

/// How many quota roots without a `QUOTA` answer are asked for separately.
/// A mailbox normally has one root; this bounds a misbehaving server.
const MAX_SEPARATE_ROOT_READS: usize = 4;

/// One resource: what is used against what is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaUsage {
    pub used: u64,
    pub limit: u64,
}

impl QuotaUsage {
    /// Whether `self` is closer to its limit than `other`. On a tie the
    /// smaller limit wins: it is the one that runs out first.
    fn fuller_than(&self, other: &QuotaUsage) -> bool {
        let left = u128::from(self.used) * u128::from(other.limit);
        let right = u128::from(other.used) * u128::from(self.limit);
        left > right || (left == right && self.limit < other.limit)
    }
}

/// The quota of the account's mailbox. Storage is in bytes, messages are a
/// count. `None` means the server sets no limit on that resource.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailboxQuota {
    pub storage: Option<QuotaUsage>,
    pub messages: Option<QuotaUsage>,
}

/// The outcome of asking a provider for its quota.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaProbe {
    /// The provider or server has no quota Satchel can read.
    Unsupported,
    /// Read. Both resources `None`: the mailbox has no limit.
    Read(MailboxQuota),
    /// The server supports quota, but this read failed. What was known before
    /// stays; the text is for the log only.
    Failed(String),
}

fn fuller(current: Option<QuotaUsage>, candidate: QuotaUsage) -> Option<QuotaUsage> {
    match current {
        Some(current) if !candidate.fuller_than(&current) => Some(current),
        _ => Some(candidate),
    }
}

/// The roots `GETQUOTAROOT` named for the mailbox. No roots at all means no
/// quota applies.
fn root_names(roots: &[QuotaRoot]) -> Vec<&str> {
    let mut names = Vec::new();
    for root in roots {
        for name in &root.quota_root_names {
            if !names.contains(&name.as_str()) {
                names.push(name.as_str());
            }
        }
    }
    names
}

/// Roots named in `QUOTAROOT` without a matching `QUOTA` response. RFC 9208
/// has the server send one per root, but some leave it to a `GETQUOTA`.
pub fn roots_without_quota(roots: &[QuotaRoot], quotas: &[Quota]) -> Vec<String> {
    root_names(roots)
        .into_iter()
        .filter(|name| !quotas.iter().any(|quota| quota.root_name == *name))
        .map(str::to_string)
        .collect()
}

/// Combines the answers to `GETQUOTAROOT INBOX` (and any `GETQUOTA` that
/// followed) into one reading.
///
/// A mailbox can sit under several roots — a per-user root and a domain-wide
/// one, say. For each resource the fullest one counts: that is the limit the
/// next message bounces off. A limit of 0 is read as "none set", as
/// `async-imap` itself does. Resources other than STORAGE and MESSAGE
/// (RFC 9208's MAILBOX, ANNOTATION-STORAGE, vendor atoms) are ignored.
pub fn mailbox_quota_from_imap(roots: &[QuotaRoot], quotas: &[Quota]) -> MailboxQuota {
    let names = root_names(roots);
    let applies = |quota: &&Quota| names.is_empty() || names.contains(&quota.root_name.as_str());
    let mut relevant: Vec<&Quota> = quotas.iter().filter(applies).collect();
    // A server that names its roots one way and answers under another still
    // told us about this mailbox; a reading from it beats none.
    if relevant.is_empty() {
        relevant = quotas.iter().collect();
    }

    let mut reading = MailboxQuota::default();
    for quota in relevant {
        for resource in &quota.resources {
            if resource.limit == 0 {
                continue;
            }
            match resource.name {
                QuotaResourceName::Storage => {
                    let usage = QuotaUsage {
                        used: resource.usage.saturating_mul(STORAGE_UNIT_BYTES),
                        limit: resource.limit.saturating_mul(STORAGE_UNIT_BYTES),
                    };
                    reading.storage = fuller(reading.storage, usage);
                }
                QuotaResourceName::Message => {
                    let usage = QuotaUsage {
                        used: resource.usage,
                        limit: resource.limit,
                    };
                    reading.messages = fuller(reading.messages, usage);
                }
                QuotaResourceName::Atom(_) => {}
            }
        }
    }
    reading
}

/// Whether the server's capabilities announce the QUOTA extension. RFC 9208
/// servers also list `QUOTA=RES-<name>`; either form counts.
pub fn advertises_quota(capabilities: &async_imap::types::Capabilities) -> bool {
    capabilities.iter().any(|capability| match capability {
        async_imap::types::Capability::Atom(atom) => {
            atom.eq_ignore_ascii_case("QUOTA")
                || atom
                    .get(..10)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("QUOTA=RES-"))
        }
        _ => false,
    })
}

/// Reads the quota of INBOX on a session that is already logged in. Never
/// fails the caller: the quota rides along on a session opened for something
/// else, and a server that refuses it must not cost the folder list.
pub async fn read_imap_quota<T>(
    session: &mut async_imap::Session<T>,
    operation_timeout: Duration,
) -> QuotaProbe
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let capabilities = match timeout(operation_timeout, session.capabilities()).await {
        Ok(Ok(capabilities)) => capabilities,
        Ok(Err(error)) => return QuotaProbe::Failed(format!("imap CAPABILITY failed: {error}")),
        Err(_) => return QuotaProbe::Failed("imap CAPABILITY timed out".into()),
    };
    if !advertises_quota(&capabilities) {
        return QuotaProbe::Unsupported;
    }

    let (roots, mut quotas) = match timeout(operation_timeout, session.get_quota_root("INBOX"))
        .await
    {
        Ok(Ok(answer)) => answer,
        Ok(Err(error)) => return QuotaProbe::Failed(format!("imap GETQUOTAROOT failed: {error}")),
        Err(_) => return QuotaProbe::Failed("imap GETQUOTAROOT timed out".into()),
    };
    // `async-imap` drops the tagged status of GETQUOTAROOT, so a `NO` arrives
    // as an empty answer. A real one always carries a QUOTAROOT line, even
    // for a mailbox under no root; without it the server refused.
    if roots.is_empty() {
        return QuotaProbe::Failed("imap GETQUOTAROOT answered without QUOTAROOT".into());
    }
    for root in roots_without_quota(&roots, &quotas)
        .into_iter()
        .take(MAX_SEPARATE_ROOT_READS)
    {
        match timeout(operation_timeout, session.get_quota(&root)).await {
            Ok(Ok(quota)) => quotas.push(quota),
            Ok(Err(error)) => {
                return QuotaProbe::Failed(format!("imap GETQUOTA failed: {error}"));
            }
            Err(_) => return QuotaProbe::Failed("imap GETQUOTA timed out".into()),
        }
    }
    QuotaProbe::Read(mailbox_quota_from_imap(&roots, &quotas))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

    /// Runs `read_imap_quota` against a scripted server over an in-memory
    /// stream. `answer` maps each command (without its tag) to the untagged
    /// lines the server sends before its tagged reply; `None` refuses it with
    /// `NO`. Returns the probe and the commands the client sent.
    async fn probe(
        answer: impl Fn(&str) -> Option<String> + Send + 'static,
    ) -> (QuotaProbe, Vec<String>) {
        let (client, server) = tokio::io::duplex(8192);
        let script = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = tokio::io::BufReader::new(reader);
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut commands = Vec::new();
            let mut line = String::new();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                let (tag, command) = (tag.to_string(), command.to_string());
                if command.starts_with("LOGIN") {
                    writer
                        .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                        .await
                        .unwrap();
                    continue;
                }
                commands.push(command.clone());
                let reply = match answer(&command) {
                    Some(untagged) => format!("{untagged}{tag} OK done\r\n"),
                    None => format!("{tag} NO not here\r\n"),
                };
                writer.write_all(reply.as_bytes()).await.unwrap();
            }
            commands
        });
        let mut client = async_imap::Client::new(client);
        client.read_response().await.unwrap();
        let mut session = client.login("user", "password").await.unwrap();
        let probe = read_imap_quota(&mut session, Duration::from_secs(5)).await;
        drop(session);
        (probe, script.await.unwrap())
    }

    const CAPABILITIES: &str = "* CAPABILITY IMAP4rev1 IDLE QUOTA\r\n";

    fn usage(used: u64, limit: u64) -> Option<QuotaUsage> {
        Some(QuotaUsage { used, limit })
    }

    #[tokio::test]
    async fn reads_storage_and_messages_from_one_root() {
        let (probe, commands) = probe(|command| match command {
            "CAPABILITY" => Some(CAPABILITIES.into()),
            "GETQUOTAROOT \"INBOX\"" => Some(
                "* QUOTAROOT INBOX \"\"\r\n* QUOTA \"\" (STORAGE 512000 1048576 MESSAGE 1200 50000)\r\n"
                    .into(),
            ),
            _ => None,
        })
        .await;
        assert_eq!(
            probe,
            QuotaProbe::Read(MailboxQuota {
                storage: usage(512_000 * 1024, 1_048_576 * 1024),
                messages: usage(1200, 50_000),
            })
        );
        assert_eq!(commands, ["CAPABILITY", "GETQUOTAROOT \"INBOX\""]);
    }

    #[tokio::test]
    async fn the_fullest_of_several_roots_counts() {
        // A personal root with room to spare under a domain root that is
        // nearly full: the domain is what bounces the next message.
        let (probe, _) = probe(|command| match command {
            "CAPABILITY" => Some(CAPABILITIES.into()),
            "GETQUOTAROOT \"INBOX\"" => Some(
                "* QUOTAROOT INBOX user/tom domain\r\n\
                 * QUOTA user/tom (STORAGE 100 1000)\r\n\
                 * QUOTA domain (STORAGE 9900 10000 MESSAGE 10 0)\r\n"
                    .into(),
            ),
            _ => None,
        })
        .await;
        assert_eq!(
            probe,
            QuotaProbe::Read(MailboxQuota {
                storage: usage(9900 * 1024, 10_000 * 1024),
                // A limit of 0 is no limit.
                messages: None,
            })
        );
    }

    #[tokio::test]
    async fn a_root_without_a_quota_answer_is_asked_for_separately() {
        let (probe, commands) = probe(|command| match command {
            "CAPABILITY" => Some(CAPABILITIES.into()),
            "GETQUOTAROOT \"INBOX\"" => Some("* QUOTAROOT INBOX \"User quota\"\r\n".into()),
            "GETQUOTA \"User quota\"" => Some("* QUOTA \"User quota\" (STORAGE 10 20)\r\n".into()),
            _ => None,
        })
        .await;
        assert_eq!(
            probe,
            QuotaProbe::Read(MailboxQuota {
                storage: usage(10 * 1024, 20 * 1024),
                messages: None,
            })
        );
        assert_eq!(commands.last().unwrap(), "GETQUOTA \"User quota\"");
    }

    #[tokio::test]
    async fn a_root_without_storage_reports_messages_only() {
        // RFC 9208 resources Satchel does not show are passed over.
        let (probe, _) = probe(|command| match command {
            "CAPABILITY" => Some(
                "* CAPABILITY IMAP4rev1 IMAP4rev2 QUOTA=RES-MESSAGE QUOTA=RES-MAILBOX\r\n".into(),
            ),
            "GETQUOTAROOT \"INBOX\"" => Some(
                "* QUOTAROOT INBOX \"\"\r\n* QUOTA \"\" (MESSAGE 980 1000 MAILBOX 12 100)\r\n"
                    .into(),
            ),
            _ => None,
        })
        .await;
        assert_eq!(
            probe,
            QuotaProbe::Read(MailboxQuota {
                storage: None,
                messages: usage(980, 1000),
            })
        );
    }

    #[tokio::test]
    async fn a_mailbox_under_no_root_has_no_limit() {
        let (probe, _) = probe(|command| match command {
            "CAPABILITY" => Some(CAPABILITIES.into()),
            "GETQUOTAROOT \"INBOX\"" => Some("* QUOTAROOT INBOX\r\n".into()),
            _ => None,
        })
        .await;
        assert_eq!(probe, QuotaProbe::Read(MailboxQuota::default()));
    }

    #[tokio::test]
    async fn a_server_without_quota_is_not_asked() {
        let (probe, commands) = probe(|command| match command {
            "CAPABILITY" => Some("* CAPABILITY IMAP4rev1 IDLE UIDPLUS\r\n".into()),
            _ => None,
        })
        .await;
        assert_eq!(probe, QuotaProbe::Unsupported);
        assert_eq!(commands, ["CAPABILITY"]);
    }

    #[tokio::test]
    async fn a_refused_quota_read_is_a_failure_not_an_empty_mailbox() {
        let (probe, _) = probe(|command| match command {
            "CAPABILITY" => Some(CAPABILITIES.into()),
            _ => None,
        })
        .await;
        assert!(
            matches!(&probe, QuotaProbe::Failed(text) if text.contains("GETQUOTAROOT")),
            "{probe:?}"
        );
    }

    #[test]
    fn storage_units_saturate_instead_of_wrapping() {
        let quota = Quota {
            root_name: String::new(),
            resources: vec![async_imap::types::QuotaResource {
                name: QuotaResourceName::Storage,
                usage: u64::MAX / 2,
                limit: u64::MAX,
            }],
        };
        let reading = mailbox_quota_from_imap(&[], &[quota]);
        assert_eq!(reading.storage, usage(u64::MAX, u64::MAX));
    }

    #[test]
    fn fuller_compares_ratios_without_overflow() {
        let nearly = QuotaUsage {
            used: u64::MAX - 1,
            limit: u64::MAX,
        };
        let half = QuotaUsage { used: 1, limit: 2 };
        assert!(nearly.fuller_than(&half));
        assert!(!half.fuller_than(&nearly));
        // Same ratio: the smaller limit runs out first.
        assert!(QuotaUsage { used: 1, limit: 2 }.fuller_than(&QuotaUsage { used: 2, limit: 4 }));
    }
}
