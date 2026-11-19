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
