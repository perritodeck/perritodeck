import os
import socket
import sys
import threading
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as mqtt  # noqa: E402


def fake_broker(code=0, split=False):
    srv = socket.socket()
    srv.bind(("127.0.0.1", 0))
    srv.listen(1)
    received = []

    def serve():
        conn, _ = srv.accept()
        with conn:
            received.append(conn.recv(1024))  # CONNECT
            ack = bytes([0x20, 0x02, 0x00, code])
            if split:  # the CONNACK arrives in pieces
                for b in ack:
                    conn.sendall(bytes([b]))
                    threading.Event().wait(0.02)
            else:
                conn.sendall(ack)
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

    def test_slider_value(self):
        port, received = fake_broker()
        mqtt.on_publish_value({"broker": f"127.0.0.1:{port}", "topic": "cmnd/lamp/Dimmer", "value": 30})
        for _ in range(50):
            if len(received) == 2:
                break
            threading.Event().wait(0.05)
        self.assertEqual(received[1], mqtt.publish_packet("cmnd/lamp/Dimmer", "30") + mqtt.DISCONNECT)

    def test_connack_in_pieces(self):
        port, received = fake_broker(code=4, split=True)
        with self.assertRaisesRegex(RuntimeError, "user name or password"):
            mqtt.publish(f"127.0.0.1:{port}", "t", "x")
        port, received = fake_broker(split=True)
        mqtt.publish(f"127.0.0.1:{port}", "t", "x")

    def test_short_connack_is_not_a_broker(self):
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)

        def serve():
            conn, _ = srv.accept()
            conn.recv(1024)
            conn.sendall(b"\x20\x02")
            conn.close()
            srv.close()

        threading.Thread(target=serve, daemon=True).start()
        with self.assertRaisesRegex(RuntimeError, "isn't an MQTT broker"):
            mqtt.publish(f"127.0.0.1:{srv.getsockname()[1]}", "t", "x")

    def test_client_ids_are_unique_and_short(self):
        ids = {mqtt.client_id() for _ in range(1000)}
        self.assertEqual(len(ids), 1000)
        self.assertTrue(all(len(i) <= 23 for i in ids))
        port, received = fake_broker()
        mqtt.publish(f"127.0.0.1:{port}", "t", "x")
        self.assertNotIn(f"perritodeck-{os.getpid()}".encode(), received[0])

    def test_refused(self):
        port, _ = fake_broker(code=4)
        with self.assertRaisesRegex(RuntimeError, "user name or password"):
            mqtt.publish(f"127.0.0.1:{port}", "t", "x")
        with self.assertRaises(ValueError):
            mqtt.on_publish({"broker": "", "topic": "t"})


if __name__ == "__main__":
    unittest.main()
