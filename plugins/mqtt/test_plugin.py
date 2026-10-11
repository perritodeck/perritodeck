import os
import socket
import sys
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as mqtt  # noqa: E402


def fake_broker(code=0):
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    received = []

    def serve():
        conn, _ = srv.accept()
        with conn:
            received.append(conn.recv(1024))  # CONNECT
            conn.sendall(bytes([0x20, 0x02, 0x00, code]))
            data = b""
            conn.settimeout(2)
            try:
                while True:
                    chunk = conn.recv(1024)
                    if not chunk:
                        break
                    data += chunk
            except socket.timeout:
                pass
            received.append(data)
        srv.close()

    threading.Thread(target=serve, daemon=True).start()
    return srv.getsockname()[1], received


class MqttTest(unittest.TestCase):
    def test_packets(self):
        self.assertEqual(mqtt._remaining_length(0), b"\x00")
        self.assertEqual(mqtt._remaining_length(321), b"\xc1\x02")
        self.assertEqual(mqtt.publish_packet("a/b", "hi", retain=True), b"\x31\x07\x00\x03a/bhi")
        c = mqtt.connect_packet("id", "u", "p")
        self.assertEqual(c[0], 0x10)
        self.assertIn(b"MQTT\x04\xc2", c)

    def test_publish_to_a_broker(self):
        port, received = fake_broker()
        mqtt.publish(f"127.0.0.1:{port}", "cmnd/lamp/POWER", "TOGGLE")
        for _ in range(50):
            if len(received) == 2:
                break
            threading.Event().wait(0.05)
        self.assertEqual(received[1], mqtt.publish_packet("cmnd/lamp/POWER", "TOGGLE") + mqtt.DISCONNECT)

    def test_refused(self):
        port, _ = fake_broker(code=4)
        with self.assertRaisesRegex(RuntimeError, "user name or password"):
            mqtt.publish(f"127.0.0.1:{port}", "t", "x")
        with self.assertRaises(ValueError):
            mqtt.on_publish({"broker": "", "topic": "t"})


if __name__ == "__main__":
    unittest.main()
