# Satchel privacy and data handling

Updated: 18 October 2026

Satchel is an open-source mail client maintained by tbuck-software. This notice
describes the app distributed from [the Satchel repository](https://github.com/tbuck-software/satchel).
Modified builds and third-party plugins can behave differently.

## Your accounts and Google data

When you connect Google, Satchel requests your email address and basic profile
to identify the account. It requests Gmail access to synchronize and display
messages, manage labels and message state, and send messages you compose. It
reads Google contacts, including other contacts, for contact lists and recipient
completion. Calendar access is read-only and supports calendar synchronization.

Authorization happens in your browser. Satchel receives OAuth tokens rather than
your Google password. You can refuse consent or revoke access through
[your Google account](https://myaccount.google.com/connections).

## Storage and retention

Mail, attachments, contacts, calendar data, settings and search indexes are
stored locally in the app profile. Account credentials and OAuth tokens are
encrypted with a key held in the operating system's credential store, subject
to the documented legacy desktop fallback. Message content in the SQLite
database is not encrypted by Satchel. Protect your device and backups accordingly.

Local data remains until you remove it through the app or delete its profile.
Revoking provider access stops authorized access; it does not erase previously
synchronized local copies. Backups and exported files remain until you delete
them. Logs and crash reports are kept locally; diagnostic export is initiated
by you. Review exported reports before sharing them.

## Network requests and sharing

Satchel connects directly to the accounts and services you configure. Sending a
message transfers it to your mail provider and recipients. Mail synchronization
also sends the changes you request to your provider.

Remote mail content, external links, feeds, integrations and plugins can contact
their respective services when enabled or used. Plugin permissions and the
service's own privacy practices apply. Update checks and downloads contact
GitHub, whose servers receive normal request metadata such as your IP address.

The distributed app does not upload your mailbox to a tbuck-software server.
tbuck-software does not sell Google user data, use it for advertising, or use it
to train general-purpose machine learning models.

Satchel's use and transfer of information received from Google APIs adheres to
the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including its Limited Use requirements.

## Questions

Use the [repository's issue tracker](https://github.com/tbuck-software/satchel/issues)
for general questions. Do not include messages, account tokens or private logs
in public issues. Report security issues through the private reporting route in
[SECURITY.md](SECURITY.md).
