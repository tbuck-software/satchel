# Architecture Decision Records

Eine Datei pro Entscheidung, nummeriert, nie umgeschrieben. Wird eine
Entscheidung gekippt, bekommt sie ein neues ADR, das das alte als
„ersetzt" markiert. Der historische Plan liegt in `docs/PLAN.md` und ist
keine Statusquelle.

| Nr. | Titel | Status |
|---|---|---|
| 0001 | Zielbild: persönlicher Client, Open Source, kein Multi-Tenant | angenommen |
| 0002 | Zuverlässigkeit vor Umbau: Mail-Korpus und Diagnose-Seite | angenommen |
| 0003 | Crate-Split: `satchel-core` ohne Tauri | angenommen |
| 0004 | Internationalisierung mit `vue-i18n`, Englisch als Default | angenommen |
| 0005 | Lokale HTTP-API, CLI und Relay werden entfernt | angenommen |
| 0006 | Mobile ist voll funktionstüchtig, nur mobil getrimmt | angenommen |
| 0007 | Kryptografie im Prozess statt über Binaries | angenommen |
| 0008 | Hub-Modell für Multi-Client | angenommen |
| 0009 | Speicherorte: Mail-Server, Hub, Gerät | angenommen |
| 0010 | Pairing eigener Geräte | angenommen |
| 0011 | Plugins nach Contribution-Modell | angenommen |
| 0012 | Frontend-Schichtung für Mail-Zustand | angenommen |
| 0013 | Doppelt geführte Spalten im Mail-Modell | angenommen |
| 0014 | Ausnahmen für abgelaufene Server-Zertifikate | angenommen |
| 0015 | Sync-Transport für den Hub: iroh, verschlüsseltes Ereignis-Log, vertrauter und blinder Hub | angenommen |
| 0016 | KI-Anbieter und KI-Funktionen | angenommen |
| 0017 | Der Katalog des Hauptfensters wird nach Ladezeitpunkt geteilt | angenommen |
| 0018 | Bereinigtes HTML an Übersetzungs-Engines ohne Sprachmodell | angenommen |
| 0019 | Regeln verschieben und leiten weiter, lokal und auf dem Server | vorgeschlagen |
| 0020 | Labs: Experimente hinter einem Schalter pro Gerät | vorgeschlagen |
