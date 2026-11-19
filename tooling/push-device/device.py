"""Pair a local mail watcher with Satchel for Android. Send wake signals for new inbox mail."""

import argparse
import base64
import datetime as dt
import getpass
import hashlib
import hmac
import ipaddress
import json
import os
import plistlib
import queue
import re
import secrets
import shutil
import socket
import sqlite3
import ssl
import struct
import subprocess
from contextlib import closing
import sys
import tempfile
import threading
import time
from pathlib import Path
from urllib.parse import urlsplit

import google.auth
import ifaddr
import requests
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.x509.oid import NameOID
from google.auth.transport.requests import AuthorizedSession
from imapclient import IMAPClient
from watchdog.events import FileSystemEventHandler
if sys.platform == "darwin":
    # SQLite modifies the WAL through an open descriptor. FSEvents can stay
    # silent until close; kqueue reports the write while Satchel is running.
    from watchdog.observers.kqueue import KqueueObserver as Observer
else:
    from watchdog.observers import Observer
from zeroconf import IPVersion, ServiceInfo, Zeroconf

ROOT = Path(os.environ.get("SATCHEL_PUSH_HOME", Path.home() / ".satchel-push")).resolve()
CONFIG = ROOT / "device.json"
SERVICE = "_satchel-wake._tcp.local."
SCOPES = ["https://www.googleapis.com/auth/firebase.messaging"]


def b64(value):
    return base64.urlsafe_b64encode(value).decode().rstrip("=")


def decode(value):
    return base64.urlsafe_b64decode(value + "=" * (-len(value) % 4))


def save(config):
    ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode="w", dir=ROOT, delete=False) as out:
        json.dump(config, out)
        out.flush()
        os.fsync(out.fileno())
    os.replace(out.name, CONFIG)


def android_config(path):
    config = json.loads(Path(path).expanduser().read_text())
    client = next((c for c in config["client"] if c["client_info"]["android_client_info"]
                   ["package_name"] == "de.tbuck.satchel"), None)
    if not client:
        raise ValueError("Choose the Android configuration for de.tbuck.satchel.")
    project = config["project_info"]
    return {"project": project["project_id"], "sender": project["project_number"],
            "app": client["client_info"]["mobilesdk_app_id"], "apiKey": client["api_key"][0]["current_key"]}


def setup_firebase():
    path = Path(os.environ.get("SATCHEL_PUSH_FIREBASE", ROOT / "google-services.json")).expanduser()
    if path.is_file():
        return android_config(path)
    if not (shutil.which("firebase") and shutil.which("gcloud")):
        print("Set up Firebase once on a computer with the Google Cloud and Firebase CLIs, "
              "or use your existing Android configuration and Google sender credentials.")
        return android_config(input("Path to google-services.json: ").strip())
    # Never reuse the user's active gcloud project implicitly. Keep the free
    # project ID across interrupted provisioning and across additional devices.
    project_file = ROOT / "project"
    project = project_file.read_text().strip() if project_file.exists() else None
    if not project:
        if input("Create a free, private Google project for push delivery? [Y/n] ").strip().lower() == "n":
            return android_config(input("Path to google-services.json: ").strip())
        project = "satchel-push-" + secrets.token_hex(5)
        project_file.write_text(project)
    if not re.fullmatch(r"[a-z][a-z0-9-]{4,28}[a-z0-9]", project):
        raise ValueError("Invalid Firebase project ID.")
    def cli(*args):
        return subprocess.check_output(list(args), text=True).strip()
    if not cli("gcloud", "auth", "list", "--filter=status:ACTIVE", "--format=value(account)"):
        subprocess.run(["gcloud", "auth", "login"], check=True)
    try:
        google.auth.default(scopes=SCOPES)
    except google.auth.exceptions.DefaultCredentialsError:
        subprocess.run(["gcloud", "auth", "application-default", "login"], check=True)
    if subprocess.run(["gcloud", "projects", "describe", project], capture_output=True).returncode:
        subprocess.run(["gcloud", "projects", "create", project, "--name=Satchel Push"], check=True)
    subprocess.run(["gcloud", "services", "enable", "firebase.googleapis.com", "fcm.googleapis.com",
                    "firebaseinstallations.googleapis.com", "apikeys.googleapis.com", "--project", project], check=True)
    # The Firebase CLI may already have its own login. No tokens travel through the phone.
    if subprocess.run(["firebase", "projects:list", "--json"], capture_output=True).returncode:
        subprocess.run(["firebase", "login"], check=True)
    projects = json.loads(cli("firebase", "projects:list", "--json"))["result"]
    if not any(p["projectId"] == project for p in projects):
        subprocess.run(["firebase", "projects:addfirebase", project], check=True)
    apps = json.loads(cli("firebase", "apps:list", "ANDROID", "--project", project, "--json"))["result"]
    app = next((a for a in apps if a.get("packageName") == "de.tbuck.satchel"), None)
    if app is None:
        app = json.loads(cli("firebase", "apps:create", "ANDROID", "Satchel", "--package-name",
                             "de.tbuck.satchel", "--project", project, "--json"))["result"]
    subprocess.run(["firebase", "apps:sdkconfig", "ANDROID", app["appId"], "--project", project,
                    "--out", str(path)], stdout=subprocess.DEVNULL, check=True)
    return android_config(path)


def mail_source():
    if os.environ.get("SATCHEL_PUSH_DB"):
        db = Path(os.environ["SATCHEL_PUSH_DB"]).expanduser()
    else:
        data = (Path.home() / "Library/Application Support" if sys.platform == "darwin"
                else Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")))
        db = data / "de.tbuck.satchel/satchel/satchel.db"
    if db.is_file() and os.environ.get("SATCHEL_PUSH_IMAP") != "1":
        print("Watching your local profile while Satchel Desktop is running.")
        return {"db": str(db.resolve())}
    accounts = []
    print("This device has no Satchel profile. Connect an IMAP inbox. Gmail can use an app password.")
    while True:
        host = input("IMAP server: ").strip()
        user = input("Email / login: ").strip()
        path = input("Path to a file containing the app password (Enter for the OS keyring): ").strip()
        account = {"host": host, "user": user}
        password = None
        if path:
            account["password_file"] = str(Path(path).expanduser().resolve(strict=True))
        else:
            password = getpass.getpass("App password: ")
        with connect_mail(account, password) as client:
            client.select_folder("INBOX", readonly=True)
        if password:
            import keyring
            keyring.set_password("satchel-push:" + host, user, password)
        accounts.append(account)
        if input("Add another inbox? [y/N] ").strip().lower() != "y":
            return {"accounts": accounts}


def connect_mail(account, password=None):
    if "password_file" in account:
        password = Path(account["password_file"]).read_text().rstrip("\r\n")
    elif password is None:
        import keyring
        password = keyring.get_password("satchel-push:" + account["host"], account["user"])
    if not password:
        raise ValueError("The IMAP password is unavailable.")
    server = urlsplit("//" + account["host"])
    if not server.hostname or server.username or server.path or server.query or server.fragment:
        raise ValueError("Use an IMAP hostname, optionally followed by its TLS port.")
    client = IMAPClient(server.hostname, port=server.port or 993, ssl=True, use_uid=False, timeout=60)
    try:
        client.login(account["user"], password)
    except Exception:
        client.shutdown()
        raise
    return client


def watch_imap(account, signal):
    # UIDNEXT advances even when a simultaneous deletion masks a new message in EXISTS.
    previous = None
    backoff = 2
    while True:
        try:
            with connect_mail(account) as client:
                idle = client.has_capability("IDLE")
                while True:
                    status = client.select_folder("INBOX", readonly=True)
                    next_uid = status.get(b"UIDNEXT")
                    if next_uid is None:
                        # Some servers omit UIDNEXT in EXAMINE. Read only the last UID.
                        count = status[b"EXISTS"]
                        last = client.fetch([count], ["UID"]) if count else {}
                        next_uid = last.get(count, {}).get(b"UID", 0) + 1
                    cursor = (status.get(b"UIDVALIDITY"), next_uid)
                    if cursor != previous:
                        signal()
                        previous = cursor
                    backoff = 2
                    if idle:
                        client.idle()
                        client.idle_check(timeout=55)
                        client.idle_done()
                    else:
                        time.sleep(45)
        except Exception as error:
            if backoff == 2:
                print(f"IMAP connection interrupted ({type(error).__name__}); retrying.", file=sys.stderr)
            time.sleep(backoff)
            backoff = min(backoff * 2, 120)


def watch_profile(path, signal):
    # Observe SQLite writes with the OS. No polling, cursor files or mail cache.
    changes = threading.Event()
    path = Path(path).resolve()
    files = {str(path), str(path) + "-wal"}

    class Changed(FileSystemEventHandler):
        def on_any_event(self, event):
            if event.event_type in ("modified", "created", "moved", "deleted") and (
                event.src_path in files or getattr(event, "dest_path", "") in files
                or (event.is_directory and event.src_path == str(path.parent))
            ):
                changes.set()

    observer = Observer()
    observer.schedule(Changed(), str(path.parent))
    observer.start()
    previous = None
    changes.set()
    try:
        while True:
            if not changes.wait(60):
                if not observer.is_alive() or not all(e.is_alive() for e in observer.emitters):
                    raise OSError("Filesystem observer stopped.")
                continue
            changes.clear()
            time.sleep(0.4)
            try:
                with closing(sqlite3.connect(path.as_uri() + "?mode=ro", uri=True)) as db:
                    db.execute("BEGIN")
                    current = db.execute("SELECT rowid, id FROM messages ORDER BY rowid DESC LIMIT 1").fetchone()
                    if current == previous:
                        continue
                    # ROWIDs can be reused after deletion. Check the new tail even
                    # if it is equal to or below the old high-water mark.
                    lower = 0
                    if previous and current and current[0] > previous[0]:
                        if db.execute("SELECT 1 FROM messages WHERE rowid=? AND id=?", previous).fetchone():
                            lower = previous[0]
                    if current and (previous is None or db.execute(
                        "SELECT 1 FROM messages m JOIN folders f ON f.id=m.folder_id "
                        "WHERE m.rowid>? AND lower(f.folder_type)='inbox' AND m.is_sent=0 "
                        "AND m.is_draft=0 LIMIT 1", (lower,)
                    ).fetchone()):
                        signal()
                    previous = current
            except sqlite3.Error:
                time.sleep(2)
                changes.set()
    finally:
        observer.stop()
        observer.join()


def sender(config):
    creds, _ = google.auth.default(scopes=SCOPES)
    session = AuthorizedSession(creds)
    url = f"https://fcm.googleapis.com/v1/projects/{config['firebase']['project']}/messages:send"

    def send(peer):
        stamp = str(int(time.time()))
        device = peer["device"]
        proof = b64(hmac.digest(decode(peer["key"]), f"{device}:{stamp}".encode(), "sha256"))
        message = {"token": peer["token"], "data": {"device": device, "time": stamp, "proof": proof},
                   "android": {"priority": "high", "ttl": "3600s", "collapse_key": "satchel-mail"}}
        if peer.get("rotated"):
            del message["token"]
            message["topic"] = "satchel_" + device
        response = session.post(url, json={"message": message}, timeout=15)
        if response.status_code == 404 and "token" in message and any(
            e.get("errorCode") == "UNREGISTERED" for e in response.json().get("error", {}).get("details", [])
        ):
            peer["rotated"] = True
            return send(peer)
        if not response.ok:
            raise RuntimeError(f"FCM rejected the wake ({response.status_code}).")
    return send


def frame(stream, obj):
    data = json.dumps(obj).encode()
    stream.sendall(struct.pack(">I", len(data)) + data)


def request(stream):
    with stream.makefile("rb") as reader:
        size, = struct.unpack(">I", reader.read(4))
        if not 1 <= size <= 8192:
            raise ValueError("Invalid frame size")
        return json.loads(reader.read(size))


def pair(config, secret, discover=None):
    if not re.fullmatch(r"[A-Za-z0-9_-]{43}", secret) or len(decode(secret)) != 32:
        raise ValueError("Invalid pairing command.")
    name = socket.gethostname().split(".")[0]
    if sys.platform == "darwin":
        pretty = subprocess.run(["/usr/sbin/scutil", "--get", "ComputerName"], capture_output=True, text=True)
        name = pretty.stdout.strip() or name
    name = name[:80]
    key = ec.generate_private_key(ec.SECP256R1())
    now = dt.datetime.now(dt.timezone.utc)
    subject = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "Satchel pairing")])
    cert = (x509.CertificateBuilder().subject_name(subject).issuer_name(subject).public_key(key.public_key())
            .serial_number(x509.random_serial_number()).not_valid_before(now - dt.timedelta(minutes=5))
            .not_valid_after(now + dt.timedelta(hours=1)).sign(key, hashes.SHA256()))
    proof = b64(hmac.digest(decode(secret), name.encode() + b"\0" + cert.public_bytes(serialization.Encoding.DER), "sha256"))
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.minimum_version = ssl.TLSVersion.TLSv1_2
    with tempfile.TemporaryDirectory() as tmp:
        pem = Path(tmp) / "identity"
        pem.write_bytes(cert.public_bytes(serialization.Encoding.PEM) + key.private_bytes(
            serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        tls.load_cert_chain(pem)
    addresses = [socket.inet_aton(ip.ip) for adapter in ifaddr.get_adapters() for ip in adapter.ips
                 if ip.is_IPv4 and not ipaddress.ip_address(ip.ip).is_loopback]
    if not addresses:
        raise ValueError("Connect the device to the same network as the phone.")
    tag = hashlib.sha256(secret.encode()).hexdigest()[:16]
    send = sender(config)
    deadline = time.monotonic() + 600
    with socket.create_server(("", 0)) as listener, Zeroconf(ip_version=IPVersion.V4Only) as dns:
        info = ServiceInfo(SERVICE, f"Satchel-{tag}.{SERVICE}", addresses=addresses,
                           port=listener.getsockname()[1], properties={"name": name, "proof": proof},
                           server=f"satchel-{tag}.local.")
        if discover:
            host, port_text = discover.rsplit(":", 1)
            target = (str(ipaddress.IPv4Address(host)), int(port_text))
            port = info.port
            proof_mac = b64(hmac.digest(decode(secret), f"announce:{port}:{proof}:{name}".encode(), "sha256"))
            announcement = json.dumps({"name": name, "port": port, "proof": proof, "mac": proof_mac}).encode()
            def announce():
                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                    while not finished.is_set():
                        try:
                            udp.sendto(announcement, target)
                        except OSError:
                            pass
                        finished.wait(2)
            finished = threading.Event()
            threading.Thread(target=announce, daemon=True).start()
        dns.register_service(info)
        print(f"Confirm {name} in Satchel on your phone.", flush=True)
        while time.monotonic() < deadline:
            listener.settimeout(max(0.1, deadline - time.monotonic()))
            raw, _ = listener.accept()
            with raw:
                raw.settimeout(10)
                try:
                    with tls.wrap_socket(raw, server_side=True) as stream:
                        req = request(stream)
                        if not secrets.compare_digest(req.get("secret", ""), secret):
                            continue
                        if req["op"] == "config":
                            if "app" not in config["firebase"]:
                                public = req.get("firebase", {})
                                if public.get("project") != config["firebase"]["project"]:
                                    raise ValueError("Firebase project mismatch.")
                                config["firebase"] = {k: public[k] for k in ("project", "sender", "app", "apiKey")}
                            nonce = str(req.get("nonce", ""))[:64]
                            known = [b64(hmac.digest(decode(p["key"]), ("reconnect:" + nonce).encode(), "sha256"))
                                     for p in config["peers"]]
                            frame(stream, {"firebase": config["firebase"], "known": known})
                        elif req["op"] == "pair":
                            peer = {k: req[k] for k in ("device", "key", "token")}
                            if not (re.fullmatch("[0-9a-f]{32}", peer["device"]) and
                                    len(decode(peer["key"])) == 32 and 16 <= len(peer["token"]) <= 4096):
                                continue
                            send(peer)
                            config["peers"] = [p for p in config["peers"] if
                                               p["device"] != peer["device"] and p["token"] != peer["token"]]
                            config["peers"].append(peer)
                            save(config)
                            frame(stream, {"ok": True})
                            if discover:
                                finished.set()
                            return
                except (OSError, ValueError, KeyError, struct.error):
                    continue
    raise TimeoutError("Pairing expired.")


def run(config):
    pending = queue.Queue(maxsize=1)

    def signal():
        try:
            pending.put_nowait(True)
        except queue.Full:
            pass

    source = config["source"]
    if "db" in source:
        def observe():
            while True:
                try:
                    watch_profile(source["db"], signal)
                except OSError as err:
                    print(f"Cannot watch the Satchel profile ({type(err).__name__}); retrying.", file=sys.stderr)
                    time.sleep(60)
        threading.Thread(target=observe, daemon=True).start()
    for account in source.get("accounts", []):
        threading.Thread(target=watch_imap, args=(account, signal), daemon=True).start()
    workers = []

    def deliver(peer, events):
        send = None
        while True:
            events.wait()
            events.clear()
            backoff = 2
            while True:
                try:
                    if send is None:
                        send = sender(config)
                    send(peer)
                    break
                except (requests.RequestException, RuntimeError, google.auth.exceptions.GoogleAuthError) as err:
                    if backoff == 2:
                        print(f"Wake delivery failed ({type(err).__name__}); retrying.", file=sys.stderr)
                    time.sleep(backoff)
                    backoff = min(300, backoff * 2)

    for peer in config["peers"]:
        event = threading.Event()
        # Each registration retries independently while its phone is unreachable.
        threading.Thread(target=deliver, args=(peer, event), daemon=True).start()
        workers.append(event)
    while True:
        pending.get()
        for event in workers:
            event.set()
        time.sleep(2)


def install_service():
    command = [sys.executable, str(Path(__file__).resolve()), "run"]
    environment = {"SATCHEL_PUSH_HOME": str(ROOT)}
    if os.getenv("GOOGLE_APPLICATION_CREDENTIALS"):
        environment["GOOGLE_APPLICATION_CREDENTIALS"] = str(Path(os.environ["GOOGLE_APPLICATION_CREDENTIALS"]).resolve())
    if sys.platform == "darwin":
        label = "de.tbuck.satchel.push-device"
        agent = Path.home() / "Library/LaunchAgents" / (label + ".plist")
        agent.parent.mkdir(exist_ok=True)
        agent.write_bytes(plistlib.dumps({"Label": label, "ProgramArguments": command, "RunAtLoad": True,
                                          "KeepAlive": True, "EnvironmentVariables": environment}))
        domain = f"gui/{os.getuid()}"
        subprocess.run(["launchctl", "bootout", domain + "/" + label], capture_output=True)
        subprocess.run(["launchctl", "bootstrap", domain, str(agent)], check=True)
    elif (sys.platform.startswith("linux") and shutil.which("systemctl") and
          subprocess.run(["systemctl", "--user", "show-environment"], stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL).returncode == 0):
        def quote(value):
            if any(ord(char) < 32 for char in value):
                raise ValueError("Invalid service path")
            return '"' + value.replace("%", "%%").replace("\\", "\\\\").replace('"', '\\"') + '"'
        directory = Path.home() / ".config/systemd/user"
        directory.mkdir(parents=True, exist_ok=True)
        unit = directory / "satchel-push.service"
        unit.write_text("[Unit]\nDescription=Satchel mail push\n[Service]\n" +
                        "".join("Environment=" + quote(k + "=" + v) + "\n" for k, v in environment.items()) +
                        "ExecStart=" + " ".join(quote(x).replace("$", "$$") for x in command) +
                        "\nRestart=always\nRestartSec=10\n[Install]\nWantedBy=default.target\n")
        subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
        subprocess.run(["systemctl", "--user", "enable", "satchel-push"], check=True)
        subprocess.run(["systemctl", "--user", "restart", "satchel-push"], check=True)
    else:
        print("Use this command in your NAS startup task. The watcher is running in this terminal now:")
        import shlex
        print(shlex.join(command), flush=True)
        os.execv(command[0], command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["pair", "run", "test"])
    parser.add_argument("secret", nargs="?")
    parser.add_argument("--project")
    parser.add_argument("--discover")
    parser.add_argument("--pair-only", action="store_true")
    args = parser.parse_args()
    os.umask(0o077)
    config = json.loads(CONFIG.read_text()) if CONFIG.exists() else None
    if args.action == "pair":
        import fcntl
        ROOT.mkdir(mode=0o700, parents=True, exist_ok=True)
        # A second installer must not overwrite the first installer's new pairing.
        lock = (ROOT / "pair.lock").open("a")
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            parser.error("Another pairing is in progress. Stop the previous command with Ctrl+C and try again.")
        config = json.loads(CONFIG.read_text()) if CONFIG.exists() else None
        if config is None:
            public = {"project": args.project} if args.project else setup_firebase()
            config = {"firebase": public, "source": mail_source(), "peers": []}
            save(config)
        if config.get("credential_path"):
            os.environ.setdefault("GOOGLE_APPLICATION_CREDENTIALS", config["credential_path"])
        try:
            google.auth.default(scopes=SCOPES)
        except google.auth.exceptions.DefaultCredentialsError:
            if shutil.which("gcloud"):
                subprocess.run(["gcloud", "auth", "application-default", "login"], check=True)
            else:
                path = Path(input("Path to this device's Google sender credential: ").strip()).expanduser()
                config["credential_path"] = str(path.resolve(strict=True))
                os.environ["GOOGLE_APPLICATION_CREDENTIALS"] = config["credential_path"]
            save(config)
        if args.project and config["firebase"]["project"] != args.project:
            raise ValueError("This device and the phone are configured for different Firebase projects.")
        pair(config, args.secret or "", args.discover)
        if not args.pair_only:
            install_service()
    elif config is None:
        parser.error("Choose Add device in Satchel first.")
    elif config.get("credential_path"):
        os.environ.setdefault("GOOGLE_APPLICATION_CREDENTIALS", config["credential_path"])
    if args.action == "test":
        send = sender(config)
        for peer in config["peers"]:
            send(peer)
    elif args.action == "run":
        run(config)


if __name__ == "__main__":
    try:
        main()
    except EOFError:
        sys.exit("Setup needs input. Run the command in an interactive terminal or use ssh -t.")
    except (ValueError, TimeoutError, RuntimeError, google.auth.exceptions.GoogleAuthError) as error:
        sys.exit(str(error))
