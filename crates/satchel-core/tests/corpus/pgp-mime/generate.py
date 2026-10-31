#!/usr/bin/env python3
"""Build the legitimate PGP/MIME signed fixtures in this directory.

Each message is shaped after one client's output and signed with the test-only
GnuPG fixture key from `../pgp`. Signatures carry a creation time, so a rerun
changes the armor but not the structure.

    python3 generate.py

GnuPG 2.4.8 with libgcrypt 1.12.0 can reject the fixture passphrase; see
`../pgp/README.md` for running against a newer libgcrypt.
"""

import os
import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
KEY_DIR = HERE.parent / "pgp"
PASSPHRASE = "fixture-passphrase"
FINGERPRINT = "C29160B0BADC8557E5FEF30C5A4E7D912785BF5C"
FROM = "Satchel GPG Fixture <gpg-fixture@example.test>"


def gpg(home, *args, data=None):
    return subprocess.run(
        ["gpg", "--homedir", home, "--batch", "--quiet", "--pinentry-mode", "loopback",
         "--passphrase", PASSPHRASE, *args],
        input=data, check=True, capture_output=True,
    ).stdout


def sign(home, entity, digest="SHA256", textmode=False):
    args = ["--local-user", FINGERPRINT, "--digest-algo", digest, "--armor", "--detach-sign"]
    if textmode:
        args.append("--textmode")
    armor = gpg(home, *args, "--output", "-", data=entity).decode("ascii")
    return "\n".join(armor.strip().splitlines())


def lines(text, eol):
    return text.strip("\n").replace("\n", eol)


def write(name, message):
    (HERE / name).write_bytes(message)


def thunderbird(home):
    """Thunderbird: preamble, protected headers, nested multipart/mixed."""
    eol = "\r\n"
    boundary = "------------sig0Tb5Kq3x7W2dL8mN4pRjYvHc"
    entity = lines("""
Content-Type: multipart/mixed; boundary="------------mixA9fQ2wE6rT1yU5iO8pZ3xC";
 protected-headers="v1"
Subject: Quarterly figures
From: Satchel GPG Fixture <gpg-fixture@example.test>
To: reader@example.test
Message-ID: <tb-signed-1@example.test>

--------------mixA9fQ2wE6rT1yU5iO8pZ3xC
Content-Type: text/plain; charset=UTF-8; format=flowed
Content-Transfer-Encoding: quoted-printable

The quarterly figures are attached. Gr=C3=BC=C3=9Fe.

--------------mixA9fQ2wE6rT1yU5iO8pZ3xC
Content-Type: text/csv; charset=UTF-8; name="figures.csv"
Content-Disposition: attachment; filename="figures.csv"
Content-Transfer-Encoding: base64

cXVhcnRlcixyZXZlbnVlCnExLDEwMApxMiwxMjAK

--------------mixA9fQ2wE6rT1yU5iO8pZ3xC--
""", eol) + eol
    signature = sign(home, entity.encode())
    message = lines(f"""
From: {FROM}
To: reader@example.test
Subject: Quarterly figures
Date: Tue, 01 Sep 2026 10:00:00 +0200
Message-ID: <tb-signed-1@example.test>
MIME-Version: 1.0
User-Agent: Mozilla Thunderbird
Content-Type: multipart/signed; micalg=pgp-sha256;
 protocol="application/pgp-signature";
 boundary="{boundary}"

This is an OpenPGP/MIME signed message (RFC 4880 and 3156)
--{boundary}
""", eol) + eol + entity + eol + lines(f"""
--{boundary}
Content-Type: application/pgp-signature; name="OpenPGP_signature.asc"
Content-Description: OpenPGP digital signature
Content-Disposition: attachment; filename="OpenPGP_signature.asc"

{signature}

--{boundary}--
""", eol) + eol
    write("thunderbird-mixed-attachment.eml", message.encode())


def apple_mail(home):
    """GPGTools in Apple Mail: folded headers, no preamble, type header last."""
    eol = "\r\n"
    boundary = "Apple-Mail=_4B1E2C3D-5F60-4A7B-8C9D-0E1F2A3B4C5D"
    entity = lines("""
Content-Transfer-Encoding: 7bit
Content-Type: text/plain;
\tcharset=us-ascii

The train leaves at nine. See you on the platform.
""", eol) + eol
    signature = sign(home, entity.encode())
    message = lines(f"""
From: {FROM}
Content-Type: multipart/signed;
\tboundary="{boundary}";
\tprotocol="application/pgp-signature";
\tmicalg=pgp-sha256
Mime-Version: 1.0 (Mac OS X Mail 16.0 \\(3774.600.62\\))
Subject: Travel plans
Message-Id: <apple-signed-1@example.test>
Date: Wed, 02 Sep 2026 09:15:00 +0200
To: reader@example.test
X-Mailer: Apple Mail (2.3774.600.62)


--{boundary}
""", eol) + eol + entity + eol + lines(f"""
--{boundary}
Content-Transfer-Encoding: 7bit
Content-Disposition: attachment;
\tfilename=signature.asc
Content-Type: application/pgp-signature;
\tname=signature.asc
Content-Description: Message signed with OpenPGP

{signature}

--{boundary}--
""", eol) + eol
    write("apple-mail-gpgtools.eml", message.encode())


def mutt(home):
    """Mutt signs in text mode and stores LF. The CRLF copy is the same mail as
    it travels over IMAP; a text-mode signature holds for both."""
    eol = "\n"
    boundary = "k3cXy7Qe2Lm9Rt4W"
    entity = lines("""
Content-Type: text/plain; charset=us-ascii
Content-Disposition: inline

The fix is on the branch.
--
Satchel GPG Fixture
""", eol) + eol
    signature = sign(home, entity.encode(), textmode=True)
    message = lines(f"""
Date: Thu, 3 Sep 2026 11:30:00 +0200
From: {FROM}
To: reader@example.test
Subject: Re: build failure
Message-ID: <mutt-signed-1@example.test>
MIME-Version: 1.0
Content-Type: multipart/signed; micalg=pgp-sha256;
\tprotocol="application/pgp-signature"; boundary="{boundary}"
Content-Disposition: inline
User-Agent: Mutt/2.2.13 (2024-03-09)

--{boundary}
""", eol) + eol + entity + eol + lines(f"""
--{boundary}
Content-Type: application/pgp-signature; name="signature.asc"
Content-Description: Digital signature

{signature}

--{boundary}--
""", eol) + eol
    write("mutt-lf.eml", message.encode())
    write("mutt-crlf.eml", message.replace("\n", "\r\n").encode())


def preamble_and_epilogue(home):
    """Both optional areas filled, SHA-512, no blank line before the close."""
    eol = "\r\n"
    boundary = "=_notes_7f3a"
    entity = lines("""
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: 8bit

Notes from today: we ship on Friday. Grüße.
""", eol) + eol
    signature = sign(home, entity.encode(), digest="SHA512")
    message = lines(f"""
From: {FROM}
To: reader@example.test
Subject: Meeting notes
MIME-Version: 1.0
Content-Type: multipart/signed; protocol="application/pgp-signature"; micalg=pgp-sha512; boundary="{boundary}"

This is a multi-part message in MIME format.
Readers without MIME support show this preamble.

--{boundary}
""", eol) + eol + entity + eol + lines(f"""
--{boundary}
Content-Type: application/pgp-signature

{signature}
--{boundary}--
This epilogue is ignored by MIME readers.
""", eol) + eol
    write("preamble-and-epilogue.eml", message.encode())


def nested_prefix_boundary(home):
    """The inner boundary extends the outer one, as some mailers generate."""
    eol = "\r\n"
    boundary = "=_b_1"
    entity = lines("""
Content-Type: multipart/alternative; boundary="=_b_12"

--=_b_12
Content-Type: text/plain; charset=us-ascii

Plain rendering of the signed note.

--=_b_12
Content-Type: text/html; charset=us-ascii

<p>HTML rendering of the signed note.</p>

--=_b_12--
""", eol) + eol
    signature = sign(home, entity.encode())
    message = lines(f"""
From: {FROM}
To: reader@example.test
Subject: Nested boundary
MIME-Version: 1.0
Content-Type: multipart/signed; protocol="application/pgp-signature"; micalg=pgp-sha256; boundary="{boundary}"

--{boundary}
""", eol) + eol + entity + eol + lines(f"""
--{boundary}
Content-Type: application/pgp-signature

{signature}

--{boundary}--
""", eol) + eol
    write("nested-prefix-boundary.eml", message.encode())


def main():
    with tempfile.TemporaryDirectory(prefix="satchel-pgp-mime-") as home:
        os.chmod(home, 0o700)
        gpg(home, "--import", str(KEY_DIR / "gpg-private.asc"))
        thunderbird(home)
        apple_mail(home)
        mutt(home)
        preamble_and_epilogue(home)
        nested_prefix_boundary(home)


if __name__ == "__main__":
    main()
