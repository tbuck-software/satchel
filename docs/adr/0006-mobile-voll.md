# 0006 Mobile ist voll funktionstüchtig, nur mobil getrimmt

Status: angenommen 2026-08-27

## Kontext

Der Desktop-Mobile-Split ist dünn (8 Dateien mit `cfg(desktop|mobile)`).
Unklar war, ob Mobile ein Lese-Client oder ein vollwertiger Client ist.

## Entscheidung

- **Alles**, was Desktop kann, kann Mobile — Compose mit Anhängen,
  Triage, Suche, Krypto (0007). Unterschiede sind Budgets, nicht
  Features.
- Android zuerst und vollständig; iOS wird danach gebaut und gegen
  Android geprüft. Zwei halbe Plattformen sind schlechter als eine ganze.
- Budgets: Bodies 30 Tage (`body_window_days`), Header unbegrenzt,
  DB-Ziel < 500 MB, Offline-Queue unbegrenzt mit Last-Write-Wins.
- Benachrichtigungen: Desktop sofort (IDLE), Mobile per Polling im
  Rahmen von WorkManager. Push kommt erst mit dem Hub (0008).

## Konsequenzen

Plugin-Runtime (0011) muss auch auf Mobile laufen oder ein
Contribution-Modell haben, das ohne Runtime auskommt.
