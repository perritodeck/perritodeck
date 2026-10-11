import os
import socket
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as wol  # noqa: E402

MAC = bytes.fromhex("1a2b3c4d5e6f")


class WolTest(unittest.TestCase):
    def setUp(self):
        self.sent = []
        wol.plugin._send = self.sent.append
        self.rx = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.rx.bind(("127.0.0.1", 0))
        self.rx.settimeout(2)
        self.port = self.rx.getsockname()[1]

    def tearDown(self):
        self.rx.close()

    def test_mac_formats(self):
        for text in ("1A:2B:3C:4D:5E:6F", "1a-2b-3c-4d-5e-6f", "1a2b.3c4d.5e6f", "1A2B3C4D5E6F", " 1a 2b 3c 4d 5e 6f "):
            self.assertEqual(wol.parse_mac(text), MAC, text)
        self.assertEqual(wol.parse_mac("a:b:c:d:e:f"), bytes([10, 11, 12, 13, 14, 15]))
        for bad in ("1A:2B:3C:4D:5E", "1A:2B:3C:4D:5E:6G", "1A2B3C4D5E6F00", "nas.lan", "1A:2B:3C:4D:5E:6F:70"):
            with self.assertRaisesRegex(ValueError, "isn't a valid MAC"):
                wol.parse_mac(bad)
        with self.assertRaisesRegex(ValueError, "enter the MAC"):
            wol.parse_mac("")

    def test_passwords(self):
        self.assertEqual(wol.parse_password(""), b"")
        self.assertEqual(wol.parse_password("00:11:22:33:44:55"), bytes.fromhex("001122334455"))
        self.assertEqual(wol.parse_password("12.34.56.78"), bytes([12, 34, 56, 78]))
        self.assertEqual(wol.parse_password("deadbeef"), bytes.fromhex("deadbeef"))
        with self.assertRaisesRegex(ValueError, "SecureOn"):
            wol.parse_password("hunter2")

    def test_wake_sends_magic_packet(self):
        self.assertIsNone(wol.wake({"mac": "1a-2b-3c-4d-5e-6f", "broadcast": "127.0.0.1", "port": str(self.port)}))
        packet = self.rx.recv(1024)
        self.assertEqual(len(packet), 102)
        self.assertEqual(packet[:6], b"\xff" * 6)
        self.assertEqual(packet[6:], MAC * 16)
        self.assertEqual(self.rx.recv(1024), packet)  # sent more than once
        self.assertTrue(any("1a:2b:3c:4d:5e:6f" in m.get("message", "") for m in self.sent))

    def test_wake_with_secureon_password_is_not_logged(self):
        wol.wake({"mac": "1A2B3C4D5E6F", "broadcast": "127.0.0.1", "port": self.port, "password": "01:02:03:04:05:06"})
        packet = self.rx.recv(1024)
        self.assertEqual(len(packet), 108)
        self.assertEqual(packet[-6:], bytes([1, 2, 3, 4, 5, 6]))
        self.assertFalse(any("01:02:03" in m.get("message", "") for m in self.sent))

    def test_errors(self):
        with self.assertRaisesRegex(ValueError, "port"):
            wol.wake({"mac": "1A2B3C4D5E6F", "port": "seven"})
        with self.assertRaisesRegex(ValueError, "port"):
            wol.wake({"mac": "1A2B3C4D5E6F", "port": 0})
        with mock.patch.object(wol.socket.socket, "sendto", side_effect=OSError(101, "Network is unreachable")):
            with self.assertRaisesRegex(RuntimeError, "couldn't send to 10.0.0.255: Network is unreachable"):
                wol.wake({"mac": "1A2B3C4D5E6F", "broadcast": "10.0.0.255"})

    def test_defaults(self):
        with mock.patch.object(wol, "send") as send:
            wol.wake({"mac": "1A2B3C4D5E6F", "port": ""})
        send.assert_called_once_with(b"\xff" * 6 + MAC * 16, "255.255.255.255", 9)


if __name__ == "__main__":
    unittest.main()
