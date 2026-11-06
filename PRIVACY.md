# Satchel privacy and data handling

Updated: 6 November 2026

Satchel is an open-source mail client maintained by tbuck-software. This notice
describes the app distributed from [the Satchel repository](https://github.com/tbuck-software/satchel).
Modified builds and third-party plugins can behave differently.

## Your accounts and Google data

When you connect Google, Satchel requests Gmail access to identify the account,
synchronize and display messages, manage labels and message state, and send
messages you compose. The initial sign-in requests only mail access. Google
Contacts access for contact photos and read-only Google Calendar access are
requested separately when you connect those features in settings. Reconnecting
an account also requests its previously granted optional permissions to keep
those features working. Existing grants for other contacts remain supported;
new connections do not request that additional permission.

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

Translation is off until you add a DeepL API key and allow an account. When
you translate a message from an allowed account, its body (text and
formatting) is sent to DeepL with your key. Every web address and link in it
(in links, images, styles, other attributes and the text itself) is replaced
by a placeholder before sending and restored on your device; e-mail addresses
written out in the text are sent as part of the text. Sender, recipients,
subject, attachments and encrypted mail are not sent. Translation
needs a DeepL API Pro key: DeepL then keeps the text only as long as the
translation takes (up to 72 hours when an error occurs) and does not use it to
improve its service. A DeepL API Free key can be saved and tested, but no mail
is sent with it, because DeepL's terms forbid personal data on the free API
and reserve the right to store what it is sent permanently (sources:
[DeepL Pro terms](https://www.deepl.com/en/pro-license), sections 3.1.2,
3.1.4, 3.3.2 and 8.3.11; [DeepL privacy policy](https://www.deepl.com/en/privacy),
section 4; read 6 November 2026). Translations are stored locally, and a local
log keeps each request for 30 days.

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
