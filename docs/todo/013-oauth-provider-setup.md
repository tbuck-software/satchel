# OAuth provider setup

This is configuration guidance, not a live task status. Release configuration is
documented in [RELEASE.md](../RELEASE.md).

## Google desktop

1. In Google Auth Platform, create an OAuth client of type **Desktop app**.
   A Web application client is unsuitable for the distributed native app.
2. Enable the Gmail, People and Calendar APIs used by Satchel in that project.
3. Download the desktop client JSON and store it in the GitHub repository secret
   `SATCHEL_GMAIL_DESKTOP_CLIENT_CONFIG`. The release workflow rejects web clients.
4. For local builds, set `SATCHEL_GMAIL_CLIENT_ID` and
   `SATCHEL_GMAIL_DESKTOP_CLIENT_SECRET` in an untracked `.env`.
5. Leave the desktop redirect URI unset. Satchel uses an ephemeral loopback
   listener and PKCE S256.
6. Configure consent-screen branding for Satchel and complete Google's required
   scope verification separately. Publishing source or binaries does not remove
   the unverified-app screen. In Testing mode, Google refresh tokens for these
   scopes expire after seven days.

Google's desktop `client_secret` is public installed-app metadata, as described
in [Google's OAuth overview](https://developers.google.com/identity/protocols/oauth2/#installed).
It is still needed by registrations that require it for code exchange and refresh.
PKCE does not make it universally optional. Confidential web-client secrets must
never be embedded in Satchel.

The app bundle identifier `de.tbuck.satchel` is separate from the OAuth client ID.
Replacing an OAuth client requires reconnecting affected accounts in Satchel;
refresh tokens cannot be transferred to a different client ID.

## Microsoft desktop

Register a public native client in Microsoft Entra and configure its loopback
redirect. Set `SATCHEL_OUTLOOK_CLIENT_ID` and, only when required by the
registration, `SATCHEL_OUTLOOK_REDIRECT_URI`. Release builds do not embed Outlook
client secrets.
