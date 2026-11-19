import multiprocessing
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

import device


class PushDeviceTest(unittest.TestCase):
    def test_inbox_insert_in_an_open_wal_wakes_the_phone(self):
        ctx = multiprocessing.get_context("spawn")
        wake = ctx.Event()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "inbox.db"
            db = sqlite3.connect(path)
            self.addCleanup(db.close)
            db.executescript("""
                PRAGMA journal_mode=WAL;
                CREATE TABLE messages(id TEXT, folder_id TEXT, is_sent INTEGER, is_draft INTEGER);
                CREATE TABLE folders(id TEXT, folder_type TEXT);
                INSERT INTO folders VALUES('in', 'inbox'), ('out', 'sent');
                INSERT INTO messages VALUES('baseline','in',0,0);
            """)
            watcher = ctx.Process(target=device.watch_profile, args=(path, wake.set))
            watcher.start()
            try:
                self.assertTrue(wake.wait(10), "watcher did not catch up at startup")
                wake.clear()
                db.execute("INSERT INTO messages VALUES('sent','out',1,0)")
                db.commit()
                self.assertFalse(wake.wait(1.5), "sent mail woke the phone")
                db.execute("INSERT INTO messages VALUES('new','in',0,0)")
                db.commit()
                self.assertTrue(wake.wait(10), "inbox insert was missed while the WAL remained open")
            finally:
                watcher.terminate()
                watcher.join()

    @patch.object(device, "save")
    @patch("builtins.input", return_value="y")
    @patch.object(device.time, "sleep")
    @patch.object(device, "Zeroconf")
    @patch.object(device.ifaddr, "get_adapters")
    @patch.object(device, "relay_post")
    def test_visible_approves_only_the_displayed_request(self, relay, adapters, dns, _sleep, prompt, save):
        link = "ab" * 16
        relay.side_effect = [
            {"id": link, "sender": "private"},
            {"phase": "requested", "code": "582014", "request": "request-for-this-code"},
            {}, {"phase": "active"},
        ]
        adapters.return_value = [Mock(ips=[Mock(is_IPv4=True, ip="192.0.2.1")])]
        config = {"source": {"accounts": [{"host": "mail.example.test", "user": "private-mailbox"}]}, "peers": []}
        device.show_on_network(config, "https://example.invalid")
        self.assertIn("582 014", prompt.call_args.args[0])
        relay.assert_any_call("https://example.invalid", "links/" + link + "/approve", "private",
                              {"request": "request-for-this-code"})
        self.assertNotIn("private-mailbox", str(relay.call_args_list))
        advertisement = dns.return_value.__enter__.return_value.register_service.call_args.args[0]
        self.assertEqual(advertisement.properties[b"id"].decode(), link)
        self.assertNotIn(b"private", advertisement.properties.values())
        self.assertEqual(config["peers"], [{"device": link, "sender": "private", "relay": "https://example.invalid"}])
        save.assert_called_once_with(config)

    @patch.object(device, "relay_post", side_effect=[{}, {"phase": "expired"}])
    @patch.object(device.google.auth, "default", side_effect=AssertionError("unexpected Google login"))
    def test_shared_sender_only_transmits_a_wake_and_stops_after_removal(self, _credentials, relay):
        send = device.sender({"peers": []})
        peer = {"relay": "https://example.invalid", "device": "ab" * 16, "sender": "test"}
        send(peer)
        relay.assert_called_once_with(peer["relay"], "links/" + peer["device"] + "/wake", peer["sender"])
        with self.assertRaises(device.PairingRemoved):
            send(peer)

    @patch.object(device, "AuthorizedSession")
    @patch.object(device.google.auth, "default", return_value=(object(), None))
    def test_rotation_falls_back_to_the_same_pairs_private_topic(self, _credentials, session):
        http = session.return_value.post
        gone = Mock(status_code=404, ok=False)
        gone.json.return_value = {"error": {"details": [{"errorCode": "UNREGISTERED"}]}}
        http.side_effect = [gone, Mock(ok=True), Mock(ok=True)]
        peer = {"device": "ab" * 16, "token": "old", "key": device.b64(bytes(32))}
        send = device.sender({"firebase": {"project": "test"}})
        send(peer)
        send(peer)
        messages = [call.kwargs["json"]["message"] for call in http.call_args_list]
        self.assertEqual(messages[0]["token"], "old")
        for message in messages[1:]:
            self.assertEqual(message["topic"], "satchel_" + peer["device"])
            self.assertNotIn("token", message)
            self.assertEqual(set(message["data"]), {"device", "time", "proof"})


if __name__ == "__main__":
    unittest.main()
