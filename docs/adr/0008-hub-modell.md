# 0008 Hub-Modell für Multi-Client

Status: angenommen 2026-08-27

## Kontext

Mehrere eigene Geräte sollen einen Zustand teilen, und manches darf nur
**einmal** passieren: geplantes Senden, Snooze-Wecken, Regeln,
Follow-up-Erinnerungen. Ein fremder Dienst ist ausgeschlossen (0001).

## Entscheidung

- Ein **Hub** ist die Autorität für Einmal-Aktionen und hält das
  Ereignis-Log für Satchel-eigenen Zustand (0009). Andere Clients
  spiegeln.
- **Eine Binary, zwei Rollen**: `satchel --headless` ist der Server
  (Docker auf NAS/VPS). Ist keiner erreichbar, übernimmt der
  Desktop-Client die Hub-Rolle per Lease — dasselbe Muster wie
  `account_sync_leases` auf Android.
- Hub-API auf **axum**, neu (0005). Transport und Vertrauen: 0010.
- Jedes Gerät spricht **selbst** mit dem Mail-Server; der Hub ist kein
  Mail-Proxy und darf ausfallen, ohne dass Mail steht.

## Konsequenzen

Setzt 0003 voraus (Core ohne Tauri). Push-Benachrichtigungen aufs Handy
werden über den Hub möglich, sind aber ein eigener Schritt.
