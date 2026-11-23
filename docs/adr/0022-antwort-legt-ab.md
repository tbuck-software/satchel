# 0022 Eine Antwort legt den Thread ab, Warten ist eine Bitte

Status: angenommen 2026-11-23

## Kontext

Bisher setzte jede gesendete Mail ihren Thread auf „Wartet“: die Antwort im
Composer (`SentMessageRepository::persist`), eine neue Mail ebenso, und die
Synchronisation tat es für Antworten aus anderen Programmen, sobald die
eigene Kopie im Gesendet-Ordner auftauchte. „Später“ zeigt Wartendes und
Wiedervorlagen. So füllte es sich mit allem, worauf nie eine Antwort kommt —
„Danke“, „Passt“, Bestätigungen — und hörte auf, eine Liste offener Punkte zu
sein.

Dazu kam der Zeitpunkt. Eine Antwort mit Undo-Fenster geht über den
Queue-Worker hinaus; der Zustandswechsel kam beim Fenster erst mit der
nächsten Synchronisation an. Der Thread verschwand dann mitten im Lesen, und
die Auswahl sprang auf die oberste Mail der Liste.

„Wartet“ hatte außerdem keine Entsprechung auf dem Server: andere Geräte
zeigten den beantworteten Thread weiter im Posteingang.

## Entscheidung

- **Eine Antwort ohne „Nachfassen“ legt den Thread als Erledigt ab**, über
  denselben Übergang wie der Erledigt-Knopf
  (`workflow::transitions::file_answered_thread`): Keyword bzw. Label, der
  Archiv-Move hinter seinem Undo-Fenster, ein Workflow-Ereignis. Nur ein
  Thread in Inbox oder Aufgabe wird abgelegt; Wartet, Wiedervorlage und
  Beiseitegelegt wurden ausdrücklich gewählt. Eine Antwort von außen
  holt den Thread wie bisher in den Posteingang zurück.
- **Mit „Nachfassen“ wartet der Thread in „Später“**, wie bisher, mit der
  Erinnerung zum gewählten Zeitpunkt.
- **Eine neue Mail** beginnt als Erledigt, mit „Nachfassen“ als Wartet. Sie
  lag nie im Posteingang; es gibt nichts auf dem Server zu verschieben.
- **Die Synchronisation ändert für eigene Mails keinen Zustand mehr.** Wer
  aus einem anderen Programm antwortet, legt dort selbst ab oder nicht.
- Mail, die Satchel von sich aus sendet (Weiterleitung einer Regel,
  `auto_submitted`), legt nichts ab.
- **Der offene Thread bleibt, bis man selbst weitergeht.** Ändert sich sein
  Zustand von außen — das Senden legt ihn ab, ein anderes Gerät, die Sync —,
  bleibt seine Zeile in der Liste, solange der Reader ihn zeigt
  (`showInReader`/`closeReader`, `activeThreadLeftList`). Ein Hinweis im
  Reader sagt, wohin er gegangen ist, mit „Zurück in die Inbox“. Erst ein
  anderer Thread, das Schließen des Readers oder ein anderer Listenumfang
  lassen ihn gehen. Eigene Aktionen wie Erledigt wählen wie bisher zuerst
  den Nachbarn.

## Folgen

- „Später“ enthält nur noch, was jemand ausdrücklich dorthin gelegt hat.
  Threads, die das alte Verhalten dort abgelegt hat, bleiben liegen; es gibt
  keine Datenreparatur.
- Beantwortete Threads verschwinden auch auf anderen Geräten und in Webmail
  aus dem Posteingang, weil Erledigt als Keyword/Label mitreist (ADR 0009).
- Eine fällige Nachfass-Erinnerung meldet sich wie bisher (Benachrichtigung,
  Markierung in „Später“); sie holt den Thread nicht in den Posteingang.
- Wer lieber alles im Posteingang lässt, hat dafür noch keine Einstellung.
