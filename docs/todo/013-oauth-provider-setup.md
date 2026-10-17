# 013 OAuth Provider Setup (Spaeter erledigen)

Status: offen  
Prioritaet: mittel

## Ziel
Google- und/oder Microsoft-OAuth in produktionsnaher Form konfigurieren und in Satchel `.env` eintragen.

## Checklist
- [ ] Google Cloud: OAuth Client fuer Web App erstellen.
- [ ] Gmail API im selben Projekt aktivieren.
- [x] Google Auth Platform: App-Zielgruppe nach dem Test auf `In production` veroeffentlichen. Im Status `Testing` laufen Refresh-Tokens bei Gmail-/Contacts-Scopes nach sieben Tagen ab. (2026-07-17)
- [ ] Redirect URI in Google setzen (dev: `http://127.0.0.1:8765/oauth/callback`).
- [ ] `SATCHEL_GMAIL_CLIENT_ID` und optional `SATCHEL_GMAIL_CLIENT_SECRET` in `.env` setzen.
- [ ] Microsoft Entra: App registrieren.
- [ ] Redirect URI fuer Web setzen (dev: `http://127.0.0.1:8765/oauth/callback`).
- [ ] `SATCHEL_OUTLOOK_CLIENT_ID` und optional `SATCHEL_OUTLOOK_CLIENT_SECRET` in `.env` setzen.
- [ ] In Satchel OAuth-Start fuer den gewuenschten Provider testen.

## Hinweise
- Bundle-Identifier (`de.tbuck.satchel`) ist getrennt von OAuth Client ID/Secret.
- Wenn spaeter `https://satchel.tbuck.de/...` als Redirect genutzt wird, muss diese URI exakt im Provider eingetragen sein.
