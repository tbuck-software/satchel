# 0001 Zielbild: persönlicher Client, Open Source, kein Multi-Tenant

Status: angenommen 2026-08-27

## Kontext

Satchel ist nach ~500 Commits in zwei Monaten ein Desktop- und
Mobile-Client mit Relay-Server, Plugin-Runtime, HTTP-API und CLI. Ohne
erklärtes Ziel lässt sich nicht entscheiden, was davon bleibt.

## Entscheidung

- Satchel ist ein **persönlicher Daily Driver** mit Open-Source-Ambition:
  Code und Doku sind so geschrieben, dass Fremde beitragen können.
- **Desktop und Mobile** sind gleichwertige Zielplattformen (siehe 0006).
- **Kein Multi-Tenant, kein gehosteter Dienst.** Alles, was einen Server
  braucht, ist Self-Hosting und läuft auch ohne (siehe 0008).
- Der Stack bleibt: Vue 3, Tauri 2, Rust, SQLite/FTS5.

## Konsequenzen

Auth, Abrechnung, Mandantentrennung gehören nicht in die Architektur.
Was nur für einen Dienst Sinn ergibt (Relay mit Postgres/APNs), fällt weg.
