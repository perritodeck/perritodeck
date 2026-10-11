"""Wake-on-LAN: send a magic packet to switch a computer on. Standard library only.

A magic packet is 6 bytes of 0xFF followed by the MAC address 16 times, sent as a UDP
broadcast (port 9 by default, sometimes 7). Some network cards also want a 4- or
6-byte "SecureOn" password appended.
"""

import os
import re
import socket
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()

DEFAULT_BROADCAST = "255.255.255.255"
SENDS = 3  # UDP can drop a packet; sending a few is what most tools do


def parse_hex_bytes(text, sizes, what):
    """Accept aa:bb:cc:dd:ee:ff, aa-bb-…, aabb.ccdd.eeff, aabbccddeeff or a:b:c:d:e:f."""
    text = (text or "").strip()
    groups = re.split(r"[:\-\s]", text)
    if len(groups) in sizes and all(re.fullmatch(r"[0-9A-Fa-f]{1,2}", g) for g in groups):
        return bytes(int(g, 16) for g in groups)
    plain = re.sub(r"[.:\-\s]", "", text)
    if len(plain) in (2 * n for n in sizes) and re.fullmatch(r"[0-9A-Fa-f]+", plain):
        return bytes.fromhex(plain)
    raise ValueError(f"“{text}” isn't a valid {what}" if text else f"enter the {what}")


def parse_mac(text):
    return parse_hex_bytes(text, (6,), "MAC address (like 1A:2B:3C:4D:5E:6F)")


def parse_password(text):
    text = (text or "").strip()
    if not text:
        return b""
    if re.fullmatch(r"\d{1,3}(\.\d{1,3}){3}", text) and all(int(p) < 256 for p in text.split(".")):
        return bytes(int(p) for p in text.split("."))  # the 4-byte form, written like an IP address
    return parse_hex_bytes(text, (4, 6), "SecureOn password (6 bytes like a MAC address)")


def magic_packet(mac, password=b""):
    return b"\xff" * 6 + mac * 16 + password


def send(packet, broadcast=DEFAULT_BROADCAST, port=9):
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        for _ in range(SENDS):
            s.sendto(packet, (broadcast, port))


@plugin.action("wake")
def wake(settings):
    mac = parse_mac(settings.get("mac"))
    password = parse_password(settings.get("password"))
    broadcast = str(settings.get("broadcast") or "").strip() or DEFAULT_BROADCAST
    raw_port = settings.get("port")
    try:
        port = 9 if raw_port is None or str(raw_port).strip() == "" else int(str(raw_port).strip())
    except ValueError:
        raise ValueError(f"“{settings.get('port')}” isn't a port number") from None
    if not 0 < port < 65536:
        raise ValueError(f"{port} isn't a port number")
    try:
        send(magic_packet(mac, password), broadcast, port)
    except OSError as e:
        raise RuntimeError(f"couldn't send to {broadcast}: {e.strerror or e}") from None
    plugin.log(f"sent a wake-up packet to {mac.hex(':')} via {broadcast}:{port}")
    return None


if __name__ == "__main__":
    plugin.run()
