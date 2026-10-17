# 0010 Pairing eigener Geräte

Status: angenommen 2026-08-27

## Entscheidung

- Jedes Gerät hat ein Schlüsselpaar. Pairing per **QR/Code** am Hub;
  das Zertifikat des Hubs wird dabei gepinnt (self-signed reicht).
- Transport über HTTPS des Hubs; im vertrauten Netz (Tailscale/WireGuard)
  entfällt nichts, es wird nur einfacher erreichbar.
- Beim Pairing werden **optional** übertragen, jeweils abwählbar:
  Kontodaten inkl. Passwörter, private PGP/S/MIME-Schlüssel. Danach nie
  wieder — nur beim Pairing.
- OAuth-Tokens werden nicht übertragen (0009).

## Konsequenzen

Bedrohungsmodell: Wer beim Pairing zusieht oder den QR fotografiert,
bekommt alles. Der Pairing-Code ist deshalb kurzlebig und einmalig.
