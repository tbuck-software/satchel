"""Exercise the installed IMAP client and TLS runtime, including Python 3.14."""
import datetime as dt
import multiprocessing
import os
import socket
import ssl
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import patch

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID

from device import watch_imap


def tls_server(directory):
    key = ec.generate_private_key(ec.SECP256R1())
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")])
    now = dt.datetime.now(dt.timezone.utc)
    cert = (x509.CertificateBuilder().subject_name(name).issuer_name(name)
            .public_key(key.public_key()).serial_number(1).not_valid_before(now - dt.timedelta(hours=1))
            .not_valid_after(now + dt.timedelta(hours=1))
            .add_extension(x509.SubjectAlternativeName([x509.DNSName("localhost")]), critical=False)
            .add_extension(x509.BasicConstraints(ca=True, path_length=None), critical=True)
            .sign(key, hashes.SHA256()))
    ca = directory / "test-ca.pem"
    ca.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    identity = directory / "identity.pem"
    identity.write_bytes(key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                                           serialization.NoEncryption()))
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(ca, identity)
    return ca, context


def serve_inbox(listener, context, commands, finished):
    next_uid = 2
    with listener, context.wrap_socket(listener.accept()[0], server_side=True) as wire, wire.makefile("rb") as reader:
        def say(line):
            wire.sendall(line.encode() + b"\r\n")
        say("* OK ready")
        while line := reader.readline():
            tag, op, *_ = line.decode().strip().split()
            commands.append(op)
            if op == "CAPABILITY":
                say("* CAPABILITY IMAP4rev1 IDLE")
            elif op == "EXAMINE":
                say("* FLAGS (\\Seen)")
                say("* 1 EXISTS")
                say("* 0 RECENT")
                say("* OK [UIDVALIDITY 1] valid")
                say(f"* OK [UIDNEXT {next_uid}] next")
            elif op == "IDLE":
                say("+ ready")
                next_uid += 1
                say("* 1 EXISTS")  # One deletion and one arrival, same size.
                if reader.readline().strip() != b"DONE":
                    return
            elif op != "LOGIN":
                raise ValueError(f"Unexpected IMAP command {op}")
            say(tag + " OK completed")
            if op == "EXAMINE" and next_uid > 2:
                finished.wait(10)
                return


def observe(account, pipe):
    watch_imap(account, lambda: pipe.send(True))


class ImapTest(unittest.TestCase):
    def test_idle_observes_an_arrival_without_fetching_contents_or_writing_a_cursor(self):
        ctx = multiprocessing.get_context("spawn")
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            ca, tls = tls_server(directory)
            password = directory / "password"
            password.write_text("fixture")
            before = {p.name: p.stat().st_mtime_ns for p in directory.iterdir()}
            server = socket.create_server(("127.0.0.1", 0))
            commands = []
            finished = threading.Event()
            thread = threading.Thread(target=serve_inbox, args=(server, tls, commands, finished), daemon=True)
            thread.start()
            account = {"host": f"localhost:{server.getsockname()[1]}", "user": "fixture", "password_file": str(password)}
            read, write = ctx.Pipe(duplex=False)
            watcher = ctx.Process(target=observe, args=(account, write))
            with patch.dict(os.environ, SSL_CERT_FILE=str(ca)):
                watcher.start()
            try:
                for _ in range(2):
                    self.assertTrue(read.poll(10), "expected a wake after connecting and after the IDLE event")
                    self.assertTrue(read.recv())
                self.assertIn("IDLE", commands)
                self.assertNotIn("FETCH", commands)
                self.assertEqual(before, {p.name: p.stat().st_mtime_ns for p in directory.iterdir()})
            finally:
                watcher.terminate()
                watcher.join()
                finished.set()
                thread.join(2)
                read.close()
                write.close()
