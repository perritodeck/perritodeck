# Wake-on-LAN

Switch on a computer or server on your network with a key press, by sending it a
Wake-on-LAN "magic packet".

## Setup

Turn on Wake-on-LAN on the computer you want to wake: in its BIOS/UEFI (often "Wake on
LAN", "Power on by PCI-E" or "Resume by LAN") and, on Windows, in the network adapter's
properties (Power Management: "Allow this device to wake the computer"; Advanced: "Wake
on Magic Packet"). Its MAC address is on its network settings page, in `ipconfig /all`
(Windows), `ip link` (Linux) or your router's list of devices.

## Actions

| Action | Fields |
|---|---|
| Wake a computer | MAC address; broadcast address (default `255.255.255.255`); port 9 or 7; SecureOn password (optional) |

MAC addresses can be written `1A:2B:3C:4D:5E:6F`, `1a-2b-3c-4d-5e-6f`, `1a2b.3c4d.5e6f`
or `1A2B3C4D5E6F`. The SecureOn password, which only a few network cards use, is 6 bytes
written like a MAC address (4 bytes like `12.34.56.78` also works); it's never logged.

If the default broadcast doesn't reach the computer (for example, the PerritoDeck
computer has several network adapters), use your network's broadcast address, such as
`192.168.1.255`. The packet is sent three times over UDP.

## Values

None.

## Not tested yet

Waking a real computer: the tests receive the packet on 127.0.0.1 and check its bytes.
