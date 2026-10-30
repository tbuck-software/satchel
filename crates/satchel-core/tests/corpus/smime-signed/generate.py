#!/usr/bin/env python3
"""Build the legitimate S/MIME signed fixtures in this directory.

The signer is a throwaway self-signed certificate minted per run; its private
key never leaves the temporary directory. Each message carries the certificate,
so the signature maths verifies without any trust anchor.

    python3 generate.py [file.eml ...]

Without arguments every fixture is rebuilt; with arguments only those.
"""

import pathlib
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
EMAIL = "smime-fixture@example.test"
BOUNDARY = "----=_smime_7c41d2e09ab3"

PLAIN = (
    "Content-Type: text/plain; charset=us-ascii\r\n"
    "Content-Transfer-Encoding: 7bit\r\n"
    "\r\n"
    "The contract is ready for signature on Monday.\r\n"
).encode()

HTML = (
    "Content-Type: text/html; charset=us-ascii\r\n"
    "Content-Transfer-Encoding: 7bit\r\n"
    "\r\n"
    "<html><body><p>Please review the invoice at "
    "<a href=\"https://bank.example/invoice/42\">the bank portal</a> "
    "before Friday.</p></body></html>\r\n"
).encode()

OPAQUE = (
    "Content-Type: text/plain; charset=us-ascii\r\n"
    "Content-Transfer-Encoding: 7bit\r\n"
    "\r\n"
    "The opaque contract text is signed as a whole.\r\n"
).encode()


def openssl(*args):
    return subprocess.run(["openssl", *args], check=True, capture_output=True).stdout


def base64_lines(der):
    return subprocess.run(
        ["base64", "-w", "64"], input=der, check=True, capture_output=True
    ).stdout.decode().strip().replace("\n", "\r\n")


def headers(subject, content_type):
    return (
        f"From: Satchel S/MIME Fixture <{EMAIL}>\r\n"
        "To: reader@example.test\r\n"
        f"Subject: {subject}\r\n"
        "Date: Fri, 04 Sep 2026 14:00:00 +0200\r\n"
        "MIME-Version: 1.0\r\n"
        f"{content_type}\r\n"
        "\r\n"
    )


def detached(subject, entity, der):
    """Detached `multipart/signed`, CRLF, with a preamble."""
    return headers(
        subject,
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\";\r\n"
        f" micalg=sha-256; boundary=\"{BOUNDARY}\"",
    ).encode() + (
        "This is an S/MIME signed message\r\n"
        "\r\n"
        f"--{BOUNDARY}\r\n"
    ).encode() + entity + (
        f"\r\n--{BOUNDARY}\r\n"
        "Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n"
        "Content-Transfer-Encoding: base64\r\n"
        "Content-Disposition: attachment; filename=\"smime.p7s\"\r\n"
        "\r\n"
        f"{base64_lines(der)}\r\n"
        "\r\n"
        f"--{BOUNDARY}--\r\n"
    ).encode()


def opaque(subject, der):
    """Opaque `application/pkcs7-mime` signed-data: the content is inside."""
    return headers(
        subject,
        "Content-Type: application/pkcs7-mime; smime-type=signed-data; name=\"smime.p7m\"\r\n"
        "Content-Transfer-Encoding: base64\r\n"
        "Content-Disposition: attachment; filename=\"smime.p7m\"",
    ).encode() + f"{base64_lines(der)}\r\n".encode()


def main(only):
    with tempfile.TemporaryDirectory(prefix="satchel-smime-") as tmp:
        tmp = pathlib.Path(tmp)
        openssl(
            "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "36500",
            "-keyout", str(tmp / "key.pem"), "-out", str(tmp / "cert.pem"),
            "-subj", f"/CN=Satchel S/MIME Fixture/emailAddress={EMAIL}",
            "-addext", f"subjectAltName=email:{EMAIL}",
            "-addext", "keyUsage=critical,digitalSignature",
            "-addext", "extendedKeyUsage=emailProtection",
        )

        def sign(entity, *extra):
            (tmp / "entity").write_bytes(entity)
            return openssl(
                "smime", "-sign", "-binary", "-md", "sha256", "-outform", "DER",
                "-in", str(tmp / "entity"), "-signer", str(tmp / "cert.pem"),
                "-inkey", str(tmp / "key.pem"), *extra,
            )

        fixtures = {
            "detached-signed.eml": lambda: detached("Contract", PLAIN, sign(PLAIN)),
            "detached-signed-html.eml": lambda: detached("Invoice", HTML, sign(HTML)),
            "opaque-signed.eml": lambda: opaque(
                "Contract (opaque)", sign(OPAQUE, "-nodetach")
            ),
        }
        for name, build in fixtures.items():
            if not only or name in only:
                (HERE / name).write_bytes(build())


if __name__ == "__main__":
    main(set(sys.argv[1:]))
