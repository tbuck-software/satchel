#!/usr/bin/env python3
"""Build the legitimate S/MIME signed fixture in this directory.

The signer is a throwaway self-signed certificate minted per run; its private
key never leaves the temporary directory. The message carries the certificate,
so the signature maths verifies without any trust anchor.

    python3 generate.py
"""

import pathlib
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
EMAIL = "smime-fixture@example.test"
BOUNDARY = "----=_smime_7c41d2e09ab3"


def openssl(*args):
    return subprocess.run(["openssl", *args], check=True, capture_output=True).stdout


def main():
    entity = (
        "Content-Type: text/plain; charset=us-ascii\r\n"
        "Content-Transfer-Encoding: 7bit\r\n"
        "\r\n"
        "The contract is ready for signature on Monday.\r\n"
    ).encode()
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
        (tmp / "entity").write_bytes(entity)
        der = openssl(
            "smime", "-sign", "-binary", "-md", "sha256", "-outform", "DER",
            "-in", str(tmp / "entity"), "-signer", str(tmp / "cert.pem"),
            "-inkey", str(tmp / "key.pem"),
        )
    signature = subprocess.run(
        ["base64", "-w", "64"], input=der, check=True, capture_output=True
    ).stdout.decode().strip().replace("\n", "\r\n")
    message = (
        f"From: Satchel S/MIME Fixture <{EMAIL}>\r\n"
        "To: reader@example.test\r\n"
        "Subject: Contract\r\n"
        "Date: Fri, 04 Sep 2026 14:00:00 +0200\r\n"
        "MIME-Version: 1.0\r\n"
        "Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\";\r\n"
        f" micalg=sha-256; boundary=\"{BOUNDARY}\"\r\n"
        "\r\n"
        "This is an S/MIME signed message\r\n"
        "\r\n"
        f"--{BOUNDARY}\r\n"
    ).encode() + entity + (
        f"\r\n--{BOUNDARY}\r\n"
        "Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n"
        "Content-Transfer-Encoding: base64\r\n"
        "Content-Disposition: attachment; filename=\"smime.p7s\"\r\n"
        "\r\n"
        f"{signature}\r\n"
        "\r\n"
        f"--{BOUNDARY}--\r\n"
    ).encode()
    (HERE / "detached-signed.eml").write_bytes(message)


if __name__ == "__main__":
    main()
