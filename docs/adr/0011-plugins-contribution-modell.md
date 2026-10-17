# 0011 Plugins nach Contribution-Modell

Status: angenommen 2026-08-27 — schreibt die heutige Runtime (`run-command`, `PluginManager.vue`) als Prototyp ab

## Kontext

Die gesamte WIT-Schnittstelle ist `run-command(id, payload)`. Die vier
Beispiel-Plugins bräuchten Hooks, die es nicht gibt. wasmtime ist
desktop-only und kostet Bauzeit. Ziel bleibt: Wer will, macht sich
Satchel zu dem, was er oder sie braucht.

## Entscheidung

Nach dem Vorbild VS Code:

1. **Contribution Points**, deklarativ im Plugin-Manifest: Kommandos,
   Sidebar-Einträge, Thread-Panels, Compose-Buttons, Einstellungen.
   Satchel rendert sie selbst — kein Plugin-Code im UI-Thread.
2. **Hooks** für Code, in dieser Priorität: Mail-Pipeline
   (on-message-received: taggen, verschieben, umschreiben) > neue
   Quellen/Provider > UI-Slots mit eigenem Inhalt > Kommandos.
3. **Activation Events**: Plugin-Code läuft erst, wenn ein Hook feuert.
4. **Versionierte, schmale API**; Zugriff nur darüber.
5. Runtime: WASM für Backend-Hooks bleibt richtig, muss aber auf Mobile
   laufen (0006). Eigene Plugin-UI, falls je nötig, in sandboxed iframes.

Erst das Interface entwerfen und mit zwei echten Plugins prüfen, dann die
Runtime bauen. Bis dahin ist die bestehende Runtime hinter einem
Feature-Flag aus dem Default-Build.

## Konsequenzen

Letzte Phase des Backlogs; hängt an 0003 (Hooks leben im Core) und 0006.
