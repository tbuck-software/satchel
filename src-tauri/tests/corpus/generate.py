import os, json, base64, quopri

D = os.path.join(os.path.dirname(os.path.abspath(__file__)))
os.makedirs(D, exist_ok=True)


def write(name, raw, expect):
    with open(os.path.join(D, name + ".eml"), "wb") as f:
        f.write(raw)
    with open(os.path.join(D, name + ".expect.json"), "w", encoding="utf-8") as f:
        json.dump(expect, f, ensure_ascii=False, indent=2)
        f.write("\n")


def crlf(s):
    return s.replace("\n", "\r\n").encode("ascii")


# ---------------------------------------------------------------- 01
h = crlf("""From: Joerg Ahlers <joerg.ahlers@example.de>
To: empfang@example.de
Subject: =?ISO-8859-1?Q?Angebot_f=FCr_die_B=FCrom=F6bel?=
Date: Tue, 04 Mar 2025 09:12:44 +0100
Message-ID: <corpus-01-iso88591@example.de>
MIME-Version: 1.0
Content-Type: text/plain; charset=ISO-8859-1
Content-Transfer-Encoding: 8bit

""")
body01 = "Sehr geehrte Damen und Herren,\n\nanbei das Angebot für die Büromöbel. Die Größe der Schränke\nhaben wir wie besprochen angepasst. Lieferung Ende März.\n\nMit freundlichen Grüßen\nJörg Ahlers\n".replace("\n", "\r\n").encode("iso-8859-1")
write("01-iso8859-1-declared", h + body01, {
    "subject": "Angebot für die Büromöbel",
    "charset": "iso-8859-1",
    "body_contains": ["Büromöbel", "Größe der Schränke", "Grüßen"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 02
h = crlf("""From: Support Team <support@example.com>
To: kunde@example.de
Subject: Ihre Anfrage
Date: Wed, 05 Mar 2025 14:03:00 +0100
Message-ID: <corpus-02-ascii-lie@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=us-ascii
Content-Transfer-Encoding: 8bit

""")
body02 = "Guten Tag,\n\nwir haben Ihre Anfrage erhalten und pruefen sie derzeit.\nDie Rueckmeldung erfolgt spätestens am Montag früh.\n\nViele Grüße\nIhr Support-Team\n".replace("\n", "\r\n").encode("utf-8")
write("02-us-ascii-mislabeled-utf8", h + body02, {
    "charset": "us-ascii",
    "body_contains": ["spätestens", "früh", "Grüße"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 03
h = crlf("""From: =?UTF-8?Q?Katharina_M=C3=BCller?= <k.mueller@example.de>
To: projekt@example.de
Subject: =?UTF-8?Q?Zusammenfassung_der_Ma=C3=9Fnahmen_zur?=
 =?UTF-8?Q?_Qualit=C3=A4tssicherung_im_M=C3=A4rz?=
Date: Thu, 06 Mar 2025 08:30:11 +0100
Message-ID: <corpus-03-split-encoded-word@example.de>
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: quoted-printable

""")
raw03 = "Hallo zusammen,\n\ndie Maßnahmen zur Qualitätssicherung sind abgeschlossen. Die Prüfprotokolle\nliegen im Ordner „März 2025“. Rückfragen bitte an mich.\n\nGrüße\nKatharina\n".encode("utf-8")
body03 = quopri.encodestring(raw03).replace(b"\n", b"\r\n")
write("03-encoded-word-split-lines", h + body03, {
    "subject": "Zusammenfassung der Maßnahmen zur Qualitätssicherung im März",
    "charset": "utf-8",
    "body_contains": ["Maßnahmen", "Qualitätssicherung", "Prüfprotokolle"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 04
boundary = "----=_Part_4711_corpus"
text04 = "Rechnung 2025-0042\r\n\r\nBetrag: 1.249,00 EUR\r\nFällig: 31. März 2025\r\n\r\nBei Rückfragen erreichen Sie uns unter 030 / 123 456.\r\n".encode("utf-8")
html04 = ("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1252\"></head>"
          "<body><h1>Rechnung 2025-0042</h1><p>Betrag: <strong>1.249,00 €</strong></p>"
          "<p>Fällig: 31. März 2025</p>"
          "<p>Bei Rückfragen erreichen Sie uns unter 030 – 123 456.</p></body></html>").encode("windows-1252")

parts = []
parts.append(crlf("From: Buchhaltung <rechnung@example.de>\nTo: kunde@example.de\nSubject: =?UTF-8?B?"))
parts.append(base64.b64encode("Rechnung 2025-0042 – fällig 31. März".encode("utf-8")))
parts.append(crlf("""?=
Date: Fri, 07 Mar 2025 11:45:20 +0100
Message-ID: <corpus-04-multipart-alt@example.de>
MIME-Version: 1.0
Content-Type: multipart/alternative; boundary="%s"

--%s
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: 8bit

""" % (boundary, boundary)))
parts.append(text04)
parts.append(crlf("\n--%s\nContent-Type: text/html; charset=windows-1252\nContent-Transfer-Encoding: 8bit\n\n" % boundary))
parts.append(html04)
parts.append(crlf("\n--%s--\n" % boundary))
write("04-multipart-alt-windows1252-html", b"".join(parts), {
    "subject": "Rechnung 2025-0042 – fällig 31. März",
    # The first two live only in the utf-8 text part, the last two only in the
    # windows-1252 html part — so this case pins both halves separately.
    "body_contains": ["Fällig: 31. März 2025", "030 / 123 456", "1.249,00 €", "030 – 123 456"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 05
h = crlf("""From: Hausverwaltung <verwaltung@example.de>
To: mieter@example.de
Subject: =?ISO-8859-1?Q?Hausordnung_ge=E4ndert?=
Date: Sat, 08 Mar 2025 16:00:00 +0100
Message-ID: <corpus-05-no-charset@example.de>
MIME-Version: 1.0
Content-Type: text/plain
Content-Transfer-Encoding: 8bit

""")
body05 = "Liebe Mieterinnen und Mieter,\n\ndie Hausordnung wurde geändert. Die Ruhezeiten gelten künftig auch\nsamstags zwischen 13 und 15 Uhr. Der Aushang hängt im Treppenhaus.\n\nMit freundlichem Gruß\nIhre Hausverwaltung\n".replace("\n", "\r\n").encode("iso-8859-1")
write("05-no-charset-8bit-latin1", h + body05, {
    "subject": "Hausordnung geändert",
    "body_contains": ["Hausordnung"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 06
parts = [crlf("From: Team <team@example.com>\nTo: alle@example.com\nSubject: =?UTF-8?B?")]
parts.append(base64.b64encode("Wöchentliches Update".encode("utf-8")))
parts.append(crlf("""?=
Date: Mon, 10 Mar 2025 07:00:00 +0100
Message-ID: <corpus-06-baseline@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: base64

"""))
b64 = base64.b64encode("Guten Morgen,\n\ndas wöchentliche Update steht im Wiki. Änderungen sind markiert.\n\nGrüße\nDas Team\n".encode("utf-8")).decode("ascii")
parts.append(("\r\n".join(b64[i:i + 76] for i in range(0, len(b64), 76)) + "\r\n").encode("ascii"))
write("06-baseline-utf8-base64", b"".join(parts), {
    "subject": "Wöchentliches Update",
    "charset": "utf-8",
    "body_contains": ["wöchentliche Update", "Änderungen", "Grüße"],
    "allow_replacement": False,
})

def wrap(b64):
    return "\r\n".join(b64[i:i + 76] for i in range(0, len(b64), 76))


# ---------------------------------------------------------------- 07
# Base64 without the closing padding, as a truncating MTA leaves it.
body07 = "Hallo Welt, das ist ein Body."
# One byte past a multiple of three, so the base64 really loses padding.
pdf07 = b"%PDF-1.4\n% ein winziges Dokument\n" * 4 + b"%"
raw07 = crlf("""From: Bounce Generator <mailer@example.com>
To: empfang@example.de
Subject: Rechnung ohne Padding
Date: Tue, 04 Mar 2025 09:12:44 +0100
Message-ID: <corpus-07-unpadded@example.com>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="=_pad_1"

--=_pad_1
Content-Type: text/plain; charset=UTF-8
Content-Transfer-Encoding: base64

""") + crlf(wrap(base64.b64encode(body07.encode()).decode().rstrip("="))) + crlf("""
--=_pad_1
Content-Type: application/pdf; name="rechnung.pdf"
Content-Disposition: attachment; filename="rechnung.pdf"
Content-Transfer-Encoding: base64

""") + crlf(wrap(base64.b64encode(pdf07).decode().rstrip("="))) + crlf("""
--=_pad_1--
""")
write("07-base64-unpadded", raw07, {
    "subject": "Rechnung ohne Padding",
    "body_contains": [body07],
    "allow_replacement": False,
    "attachments": [{"filename": "rechnung.pdf", "size": len(pdf07)}],
})

# ---------------------------------------------------------------- 08
# An empty parameter slot in Content-Type, as broken bounce generators write it.
first08 = b"Erster Anhang" * 3
second08 = b"Zweiter Anhang" * 2
raw08 = crlf("""From: Bounce Generator <mailer@example.com>
To: empfang@example.de
Subject: Leeres Segment im Content-Type
Date: Tue, 04 Mar 2025 10:00:00 +0100
Message-ID: <corpus-08-empty-segment@example.com>
MIME-Version: 1.0
Content-Type: multipart/mixed; ; boundary="x"

--x
Content-Type: text/plain; charset=UTF-8

Der Bericht liegt bei.

--x
Content-Type: application/octet-stream; name="erster.bin"
Content-Disposition: attachment; filename="erster.bin"
Content-Transfer-Encoding: base64

""") + crlf(wrap(base64.b64encode(first08).decode())) + crlf("""
--x
Content-Type: application/octet-stream; name="zweiter.bin"
Content-Disposition: attachment; filename="zweiter.bin"
Content-Transfer-Encoding: base64

""") + crlf(wrap(base64.b64encode(second08).decode())) + crlf("""
--x--
""")
write("08-content-type-empty-segment", raw08, {
    "subject": "Leeres Segment im Content-Type",
    "body_contains": ["Der Bericht liegt bei."],
    "allow_replacement": False,
    "attachments": [
        {"filename": "erster.bin", "size": len(first08)},
        {"filename": "zweiter.bin", "size": len(second08)},
    ],
})

# ---------------------------------------------------------------- 09
# The outer boundary is a prefix of the inner one.
raw09 = crlf("""From: Newsletter <news@example.com>
To: empfang@example.de
Subject: Verschachtelte Grenzen
Date: Tue, 04 Mar 2025 11:00:00 +0100
Message-ID: <corpus-09-nested@example.com>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="=_b_1"

--=_b_1
Content-Type: multipart/alternative; boundary="=_b_12"

--=_b_12
Content-Type: text/plain; charset=UTF-8

Die Textfassung der Einladung.

--=_b_12
Content-Type: text/html; charset=UTF-8

<html><body><p>Die HTML-Fassung der Einladung.</p></body></html>

--=_b_12--

--=_b_1
Content-Type: text/calendar; charset=UTF-8; method=REQUEST

BEGIN:VCALENDAR
END:VCALENDAR

--=_b_1--
""")
write("09-nested-boundary-prefix", raw09, {
    "subject": "Verschachtelte Grenzen",
    "body_contains": ["Die Textfassung der Einladung."],
    "body_excludes": ["=_b_12", "Content-Type: text/html"],
    "allow_replacement": False,
})

# ---------------------------------------------------------------- 10
# The delimiter appears inside a quoted line, and there is an epilogue.
raw10 = crlf("""From: Liste <liste@example.com>
To: empfang@example.de
Subject: Nachwort nach der Schlussgrenze
Date: Tue, 04 Mar 2025 12:00:00 +0100
Message-ID: <corpus-10-epilogue@example.com>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="=_e_1"

--=_e_1
Content-Type: text/plain; charset=UTF-8

-- 
Im Zitat stand: --=_e_1-- und danach ging es weiter.

--=_e_1
Content-Type: application/octet-stream; name="anhang.bin"
Content-Disposition: attachment; filename="anhang.bin"

ANHANGSINHALT
--=_e_1--
Nachwort, das kein Teil der Nachricht ist.
""")
write("10-epilogue-after-closing-boundary", raw10, {
    "subject": "Nachwort nach der Schlussgrenze",
    "body_contains": ["Im Zitat stand", "danach ging es weiter"],
    "body_excludes": ["Nachwort, das kein Teil"],
    "allow_replacement": False,
    "attachments": [{"filename": "anhang.bin", "size": len(b"ANHANGSINHALT")}],
})

for n in sorted(os.listdir(D)):
    print(n, os.path.getsize(os.path.join(D, n)))
