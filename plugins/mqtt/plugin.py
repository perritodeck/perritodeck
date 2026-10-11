"""Publish MQTT messages (MQTT 3.1.1, QoS 0, plain TCP). Standard library only."""

import os
import socket
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
TIMEOUT = 5


def _string(s):
    b = s.encode()
    return struct.pack("!H", len(b)) + b


def _remaining_length(n):
    out = bytearray()
    while True:
        byte, n = n % 128, n // 128
        out.append(byte | (0x80 if n else 0))
        if not n:
            return bytes(out)


def connect_packet(client_id, username=None, password=None, keepalive=30):
    flags = 0x02  # clean session
    payload = _string(client_id)
    if username:
        flags |= 0x80
        payload += _string(username)
        if password:
            flags |= 0x40
            payload += _string(password)
    variable = _string("MQTT") + bytes([4, flags]) + struct.pack("!H", keepalive)
    body = variable + payload
    return bytes([0x10]) + _remaining_length(len(body)) + body


def publish_packet(topic, payload, retain=False):
    body = _string(topic) + payload.encode()
    return bytes([0x30 | (0x01 if retain else 0)]) + _remaining_length(len(body)) + body


DISCONNECT = bytes([0xE0, 0x00])

CONNACK_ERRORS = {
    1: "the broker doesn't speak MQTT 3.1.1",
    2: "the broker refused the client id",
    3: "the broker is unavailable",
    4: "wrong user name or password",
    5: "not authorised",
}


def publish(broker, topic, payload, retain=False, username=None, password=None):
    host, _, port = broker.strip().rpartition(":") if ":" in broker else (broker.strip(), "", "1883")
    with socket.create_connection((host, int(port or 1883)), timeout=TIMEOUT) as s:
        s.sendall(connect_packet(f"perritodeck-{os.getpid()}", username, password))
        ack = s.recv(4)
        if len(ack) < 4 or ack[0] != 0x20:
            raise RuntimeError("that isn't an MQTT broker")
        if ack[3] != 0:
            raise RuntimeError(CONNACK_ERRORS.get(ack[3], f"the broker refused (code {ack[3]})"))
        s.sendall(publish_packet(topic, payload, retain) + DISCONNECT)


@plugin.action("publish")
def on_publish(settings):
    broker = (settings.get("broker") or "").strip()
    topic = (settings.get("topic") or "").strip()
    if not broker or not topic:
        raise ValueError("set the broker and the topic")
    try:
        publish(
            broker,
            topic,
            settings.get("payload") or "",
            bool(settings.get("retain")),
            settings.get("username") or None,
            settings.get("password") or None,
        )
    except OSError as e:
        raise RuntimeError(f"couldn't reach {broker}: {e.strerror or e}") from None


@plugin.action("publish-value")
def on_publish_value(settings):
    value = str(settings.get("value", ""))
    payload = settings.get("payload") or "{value}"
    return on_publish({**settings, "payload": payload.replace("{value}", value)})


if __name__ == "__main__":
    plugin.run()
