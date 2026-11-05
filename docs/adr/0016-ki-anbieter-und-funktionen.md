# 0016 KI-Anbieter und KI-Funktionen

Status: angenommen 2026-11-05

## Kontext

Satchel hat heute keine KI-Funktion. Vorbereitet ist nur die Enum-Variante
`TopicAddedBy::Llm` in `crates/satchel-core/src/db/topics.rs`, die niemand
setzt. Die KI-Funktionen in Spark und Outlook taugen in der Praxis wenig. Sie
fassen kurze Mails zusammen, die man schneller selbst liest, und schreiben
Antworten, die nicht nach dem Absender klingen.

Zwei Fragen müssen vor der ersten Zeile Code beantwortet sein. Wer bezahlt die
Anfragen, und wohin gehen dabei die Mails? Ein Abo-Login (claude.ai, ChatGPT)
wirkt billig, kann aber gegen die Bedingungen des Anbieters verstoßen und das
Konto des Nutzers kosten. Ein eigener Proxy oder ein Projekt-Abo wäre ein
gehosteter Dienst mit Abrechnung, und Mail-Inhalte liefen über unsere Server.
Damit wären wir Auftragsverarbeiter nach DSGVO. Beides schließt 0001 aus.

Mit API-Schlüssel kosten Mail-Aufgaben wenig. Nach den Listenpreisen von
Anthropic kostet Claude Haiku 5.5 $0,10 je Million Eingabe-Tokens und $0,50 je
Million Ausgabe-Tokens, Claude Sonnet 5.5 $2 bzw. $10. Bei 100 klassifizierten
Mails am Tag mit rund 1.500 Tokens hinein und 50 heraus fallen mit Haiku etwa
$0,50 im Monat an. Zehn Entwürfe oder Zusammenfassungen am Tag mit Sonnet
(4.000 bis 5.000 Tokens hinein, 500 heraus) kosten etwa $4 bis $5 im Monat.
Ein lokales Modell kostet nichts außer Strom.

## Entscheidung

### Anbieter

- Die Anbieter-Schicht lebt in `satchel-core` (0003) und läuft damit auch auf
  Mobile (0006). Sie spricht zwei Protokolle:
  1. **OpenAI-kompatible Chat Completions**, konfiguriert über Basis-URL und
     Schlüssel. Voreinstellungen gibt es für OpenAI, OpenRouter, Mistral,
     Ollama (`http://localhost:11434/v1`) und LM Studio
     (`http://localhost:1234/v1`). Die beiden letzten laufen lokal und
     brauchen keinen Schlüssel.
  2. **Anthropic Messages API**, nativ. Den OpenAI-Kompatibilitätsmodus von
     Anthropic nutzen wir nicht, weil Anthropic ihn selbst nicht für den
     Produktivbetrieb empfiehlt.
- OpenRouter ist die Antwort auf "ein Konto, alle Modelle". Einen eigenen
  Sammeldienst bauen wir nicht.
- Schlüssel liegen wie Konto-Zugangsdaten verschlüsselt mit dem Master-Key des
  Profils (`enc:v2:`, siehe README "Where the password ends up") und gehen mit
  ins Backup. Über unverschlüsseltes `http://` schickt Satchel einen Schlüssel
  nur an Loopback-Adressen.
- Antworten, die das Programm weiterverarbeitet, fordert Satchel als JSON nach
  Schema an: `response_format` mit `json_schema` bei Chat Completions,
  `output_config.format` bei Anthropic. Kann ein Anbieter oder Modell das
  nicht, fällt Satchel auf den JSON-Modus zurück. Core prüft jede Antwort
  gegen das Schema und verwirft, was nicht passt. Höchstens ein zweiter
  Versuch, keine Schleife.
- Fällt ein Anbieter aus, springt Satchel nicht auf einen anderen um. Die
  Zustimmung gilt einem Anbieter, nicht dem nächstbesten.

### Was wir nicht bauen

- **Kein claude.ai-Login.** Anthropic erlaubt Drittentwicklern nicht, ohne
  vorherige Freigabe claude.ai-Login oder claude.ai-Kontingente anzubieten,
  auch nicht für Agenten auf dem Agent SDK.
- **Kein ChatGPT-Login in diesem Schritt.** OpenAI bietet seit 2026 mit "Sign
  in with ChatGPT" einen offiziellen Weg, das Plus- oder Pro-Kontingent in
  Open-Source- und lokal betriebenen Apps zu nutzen (PKCE, Loopback-Redirect,
  kein Client-Secret). Ein Bann-Risiko ist das nach heutiger Doku nicht. Der Weg
  läuft aber über die Responses API, also ein drittes Protokoll, verlangt einen
  Login-Flow mit lokalem Listener, der auf Mobile ungeklärt ist, und bindet an
  einen Anbieter. Ob Satchel als "open-source partner" ohne Antrag zählt, lässt
  die Doku offen. Wer das anbieten will, schreibt ein eigenes ADR.
- **Keine Agent-CLIs als Standardweg.** Satchel startet nicht `claude`,
  `codex`, `opencode` oder `pi` als Unterprozess, um deren Login
  mitzubenutzen. Wer seine Mail mit einem eigenen Agenten auswerten will, liest
  die read-only Views aus 0005. Dafür gibt es sie.
- **Kein Proxy, kein Projekt-Abo, keine Abrechnung** (0001).

### Datenschutz

- KI ist ab Werk aus. Der Nutzer schaltet sie je Konto und je Funktion ein.
  Ein Dienstkonto kann aus bleiben, während das private Konto mitmacht.
- Ein Schalter "Nur lokale Modelle" lässt nur Anbieter zu, deren Host eine
  Loopback- oder private Netzadresse ist. Das entscheidet die URL, nicht der
  Name der Voreinstellung.
- Funktionen, die im Hintergrund über neue Mails laufen, brauchen einen
  lokalen Anbieter oder eine eigene Zustimmung für genau diese Funktion mit
  genau diesem Anbieter.
- Der Nutzer sieht, was das Gerät verlässt. Vor einer Anfrage auf Abruf zeigt
  Satchel, welche Teile der Mail mitgehen. Ein lokales Protokoll hält je
  Anfrage Funktion, Anbieter, Modell, Tokens und auf Wunsch den gesendeten
  Text fest.
- Ein Monatsdeckel begrenzt Tokens oder Kosten, sichtbar mit Zähler.
  Tokens zählt Satchel aus dem `usage`-Feld der Antwort. Kosten kennt es nur,
  wo der Anbieter sie meldet oder der Nutzer einen Preis je Modell einträgt. Ist
  der Deckel erreicht, pausieren die Hintergrundfunktionen, und Anfragen auf
  Abruf fragen nach.

### Funktionen, in dieser Reihenfolge

1. **Suche in natürlicher Sprache.** Das Modell übersetzt die Frage in die
   Operator-Syntax der Suche (`from:`, `subject:`, `after:`, `is:unread`, …).
   Satchel schreibt die Abfrage sichtbar und editierbar ins Suchfeld und führt
   sie lokal über FTS5 aus. Das Gerät verlässt nur die Frage, die Grammatik und
   die Namen von Ordnern und Tags, kein Mail-Inhalt.
2. **"Braucht eine Antwort?"** erkennt Mails, auf die der Nutzer antworten
   sollte, und speist Follow-ups und Smart Buckets. Läuft im Hintergrund, also
   mit den Regeln oben.
3. **Termine und Aufgaben herauslesen.** Daraus wird ein Vorschlag für einen
   Kalendertermin oder eine Aufgabe. Anlegen tut der Nutzer, nie Satchel.
4. **Themen zuordnen** mit `TopicAddedBy::Llm`. Eine Zuordnung des Nutzers
   schlägt die des Modells, und was der Nutzer entfernt, setzt das Modell
   nicht wieder.
5. **Antwortentwürfe im eigenen Stil.** Aus gesendeten Mails entsteht einmalig
   ein Stilprofil als Text, den der Nutzer lesen und ändern kann. Dafür geht
   eine Auswahl gesendeter Mails mit eigener Zustimmung an den Anbieter. Danach
   reisen nur Thread und Profil. Entwürfe landen im Composer und werden nie
   automatisch gesendet.
6. **Zusammenfassung auf Abruf**, nur für lange Threads. Für eine Mail, die in
   zehn Sekunden gelesen ist, gibt es keinen Knopf.
7. **Übersetzung** als zweite Engine hinter der Übersetzungs-Naht. DeepL kommt
   zuerst und wird getrennt gebaut.

## Konsequenzen

Die KI-Funktionen sind Core-Funktionen, keine Plugins. Ob Plugins die
Anbieter-Schicht über die Plugin-API (0011) nutzen dürfen, entscheidet ein
späteres ADR.

Prompts stehen auf Englisch in Rust. Was die Funktionen dem Nutzer sagen, geht
wie jede Meldung als Message-Key an das Frontend (0004).

Neue Tabellen für Anbieter, Zustimmungen und das Anfrage-Protokoll entstehen
per additiver Migration. Das Protokoll braucht eine Aufbewahrungsgrenze, sonst
liegt nach einem Jahr jede gesendete Mail ein zweites Mal in der Datenbank.

Wer einen Abo-Login, automatisches Senden oder einen gehosteten Dienst will,
schreibt ein neues ADR.

## Quellen

Abgerufen am 2026-11-05.

- Anthropic, Agent SDK overview, Hinweis zu claude.ai-Login für Drittprodukte:
  <https://code.claude.com/docs/en/agent-sdk/overview>
- Anthropic, Preise: <https://platform.claude.com/docs/en/about-claude/pricing>
- Anthropic, Structured outputs:
  <https://platform.claude.com/docs/en/build-with-claude/structured-outputs>
- Anthropic, OpenAI SDK compatibility ("not considered a long-term or
  production-ready solution"): <https://platform.claude.com/docs/en/api/openai-sdk>
- OpenAI, Sign in with ChatGPT:
  <https://developers.openai.com/siwc/quickstart>,
  <https://developers.openai.com/siwc/token-sharing-open-source>,
  <https://developers.openai.com/siwc/token-sharing-open-source/sign-in>
- Ollama, OpenAI compatibility:
  <https://docs.ollama.com/api/openai-compatibility>
- LM Studio, OpenAI compatibility und Structured output:
  <https://lmstudio.ai/docs/developer/openai-compat>,
  <https://lmstudio.ai/docs/developer/openai-compat/structured-output>
- OpenRouter, Structured outputs:
  <https://openrouter.ai/docs/guides/features/structured-outputs>
