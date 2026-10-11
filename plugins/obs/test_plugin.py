"""Tests against a fake obs-websocket 5 server (RFC 6455 server side + op codes)."""

import base64
import hashlib
import json
import os
import socket
import struct
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import plugin as obs  # noqa: E402

# Fast reconnects and waits for the tests.
obs.BACKOFF_FIRST = 0.05
obs.BACKOFF_MAX = 0.2
obs.CONNECT_WAIT = 0.4

GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
SALT = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI="
CHALLENGE = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="

ALL_REQUESTS = [
    "GetVersion", "GetStats", "GetSceneList", "GetStudioModeEnabled", "GetStreamStatus", "GetRecordStatus",
    "SplitRecordFile", "CreateRecordChapter", "SetCurrentProgramScene",
]


def wait_for(cond, timeout=2.0, message="condition"):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if cond():
            return
        time.sleep(0.005)
    raise AssertionError(f"timed out waiting for {message}")


class Fail(Exception):
    def __init__(self, code, comment=None):
        super().__init__(comment)
        self.code, self.comment = code, comment


def server_frame(opcode, payload, fin=True):
    """An unmasked frame, as a server sends it."""
    return obs.encode_frame(opcode, payload, fin=fin, mask=False)


class Conn:
    def __init__(self, sock):
        self.sock = sock
        self.buf = b""
        self.lock = threading.Lock()

    def read(self, n):
        while len(self.buf) < n:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise ConnectionError("client went away")
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def send(self, data):
        with self.lock:
            self.sock.sendall(data)


class FakeObs:
    """A small obs-websocket 5 server: handshake, Hello/Identify with the auth challenge,
    requests, batches and events."""

    def __init__(self, password=None, port=0):
        self.password = password
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind(("127.0.0.1", port))
        self.listener.listen(8)
        self.port = self.listener.getsockname()[1]
        self.url = f"ws://127.0.0.1:{self.port}"
        self.clients = []
        self.requests = []  # (requestType, requestData) from op 6
        self.batched = []  # (requestType, requestData) from op 8
        self.identifies = []
        self.handshakes = []
        self.pongs = []
        self.unmasked = 0
        self.connections = 0
        self.state = {
            "program": "Live", "preview": None, "studio": False, "streaming": False, "recording": False,
            "stream_bytes": 0, "available": list(ALL_REQUESTS),
        }
        self.handlers = {}
        self.running = True
        threading.Thread(target=self._accept, daemon=True).start()

    # -- plumbing --

    def _accept(self):
        while self.running:
            try:
                sock, _ = self.listener.accept()
            except OSError:
                return
            self.connections += 1
            threading.Thread(target=self._serve, args=(Conn(sock),), daemon=True).start()

    def _read_message(self, conn):
        parts = []
        while True:
            b0, b1 = conn.read(2)
            fin, opcode = b0 & 0x80, b0 & 0x0F
            if not b1 & 0x80:
                self.unmasked += 1
            n = b1 & 0x7F
            if n == 126:
                (n,) = struct.unpack("!H", conn.read(2))
            elif n == 127:
                (n,) = struct.unpack("!Q", conn.read(8))
            key = conn.read(4) if b1 & 0x80 else b"\0\0\0\0"
            payload = obs.mask_bytes(conn.read(n), key) if n else b""
            if opcode == obs.OP_PONG:
                self.pongs.append(payload)
                continue
            if opcode == obs.OP_PING:
                conn.send(server_frame(obs.OP_PONG, payload))
                continue
            if opcode == obs.OP_CLOSE:
                raise ConnectionError("client closed")
            parts.append(payload)
            if fin:
                return json.loads(b"".join(parts).decode())

    def send(self, conn, message):
        conn.send(server_frame(obs.OP_TEXT, json.dumps(message).encode()))

    def close_with(self, conn, code, reason=""):
        try:
            conn.send(server_frame(obs.OP_CLOSE, struct.pack("!H", code) + reason.encode()))
        except OSError:
            pass
        time.sleep(0.02)
        conn.sock.close()

    def _serve(self, conn):
        try:
            head = b""
            while b"\r\n\r\n" not in head:
                chunk = conn.sock.recv(4096)
                if not chunk:
                    return
                head += chunk
            lines = head.decode().split("\r\n")
            headers = {l.split(":", 1)[0].lower(): l.split(":", 1)[1].strip() for l in lines[1:] if ":" in l}
            self.handshakes.append((lines[0], headers))
            accept = base64.b64encode(hashlib.sha1((headers["sec-websocket-key"] + GUID).encode()).digest()).decode()
            conn.send(
                (
                    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                    f"Sec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n"
                ).encode()
            )
            hello = {"obsStudioVersion": "30.2.2", "obsWebSocketVersion": "5.5.2", "rpcVersion": 1}
            if self.password is not None:
                hello["authentication"] = {"challenge": CHALLENGE, "salt": SALT}
            self.send(conn, {"op": 0, "d": hello})
            identify = self._read_message(conn)
            self.identifies.append(identify)
            if self.password is not None:
                secret = base64.b64encode(hashlib.sha256((self.password + SALT).encode()).digest()).decode()
                expected = base64.b64encode(hashlib.sha256((secret + CHALLENGE).encode()).digest()).decode()
                if identify["d"].get("authentication") != expected:
                    self.close_with(conn, 4009, "Authentication failed.")
                    return
            self.send(conn, {"op": 2, "d": {"negotiatedRpcVersion": 1}})
            self.clients.append(conn)
            while True:
                message = self._read_message(conn)
                d = message["d"]
                if message["op"] == 6:
                    self.requests.append((d["requestType"], d.get("requestData")))
                    status, data = self._answer(d["requestType"], d.get("requestData") or {})
                    reply = {"requestType": d["requestType"], "requestId": d["requestId"], "requestStatus": status}
                    if data is not None:
                        reply["responseData"] = data
                    self.send(conn, {"op": 7, "d": reply})
                elif message["op"] == 8:
                    results = []
                    for r in d["requests"]:
                        self.batched.append((r["requestType"], r.get("requestData")))
                        status, data = self._answer(r["requestType"], r.get("requestData") or {})
                        item = {"requestType": r["requestType"], "requestId": r.get("requestId"), "requestStatus": status}
                        if data is not None:
                            item["responseData"] = data
                        results.append(item)
                    self.send(conn, {"op": 9, "d": {"requestId": d["requestId"], "results": results}})
        except (OSError, ConnectionError, ValueError):
            pass
        finally:
            if conn in self.clients:
                self.clients.remove(conn)
            conn.sock.close()

    def _answer(self, request_type, data):
        handler = self.handlers.get(request_type) or getattr(self, "on_" + request_type, None)
        try:
            result = handler(data) if handler else {}
        except Fail as f:
            status = {"result": False, "code": f.code}
            if f.comment:
                status["comment"] = f.comment
            return status, None
        return {"result": True, "code": 100}, result or None

    # -- default answers --

    def on_GetVersion(self, d):
        return {"obsVersion": "30.2.2", "obsWebSocketVersion": "5.5.2", "rpcVersion": 1,
                "availableRequests": self.state["available"], "supportedImageFormats": ["png", "jpg"]}

    def on_GetSceneList(self, d):
        return {"currentProgramSceneName": self.state["program"], "currentPreviewSceneName": self.state["preview"],
                "scenes": [{"sceneName": "Live", "sceneIndex": 0}]}

    def on_GetStudioModeEnabled(self, d):
        return {"studioModeEnabled": self.state["studio"]}

    def on_GetStreamStatus(self, d):
        active = self.state["streaming"]
        return {"outputActive": active, "outputReconnecting": False,
                "outputTimecode": "00:01:02.345" if active else "00:00:00.000", "outputDuration": 62345,
                "outputCongestion": 0, "outputBytes": self.state["stream_bytes"], "outputSkippedFrames": 0,
                "outputTotalFrames": 100}

    def on_GetRecordStatus(self, d):
        active = self.state["recording"]
        return {"outputActive": active, "outputPaused": False,
                "outputTimecode": "01:00:00.000" if active else "00:00:00.000", "outputDuration": 0, "outputBytes": 0}

    def on_GetReplayBufferStatus(self, d):
        raise Fail(604, "Replay buffer is not available.")

    def on_GetVirtualCamStatus(self, d):
        return {"outputActive": False}

    def on_GetCurrentSceneTransition(self, d):
        return {"transitionName": "Cut", "transitionKind": "cut_transition", "transitionFixed": True,
                "transitionDuration": None, "transitionConfigurable": False, "transitionSettings": None}

    def on_GetProfileList(self, d):
        return {"currentProfileName": "Untitled", "profiles": ["Untitled"]}

    def on_GetSceneCollectionList(self, d):
        return {"currentSceneCollectionName": "Main", "sceneCollections": ["Main"]}

    def on_GetStats(self, d):
        return {"cpuUsage": 3.14159, "memoryUsage": 512.7, "availableDiskSpace": 1000, "activeFps": 60.0000001,
                "averageFrameRenderTime": 1.2, "renderSkippedFrames": 1, "renderTotalFrames": 900,
                "outputSkippedFrames": 2, "outputTotalFrames": 100, "webSocketSessionIncomingMessages": 1,
                "webSocketSessionOutgoingMessages": 1}

    def on_SetCurrentProgramScene(self, d):
        if d.get("sceneName") == "Nope":
            raise Fail(600, "No source was found by the name of `Nope`.")
        return None

    # -- driving it from tests --

    def event(self, event_type, data, intent=1):
        for conn in list(self.clients):
            self.send(conn, {"op": 5, "d": {"eventType": event_type, "eventIntent": intent, "eventData": data}})

    def drop_all(self):
        """Like OBS crashing: connections vanish without a close frame."""
        for conn in list(self.clients):
            try:
                conn.sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            conn.sock.close()

    def kick(self, code=4011):
        for conn in list(self.clients):
            self.close_with(conn, code)

    def stop(self):
        self.running = False
        try:
            self.listener.shutdown(socket.SHUT_RDWR)  # wakes accept(); close() alone may not
        except OSError:
            pass
        try:
            self.listener.close()
        except OSError:
            pass
        self.drop_all()


class PluginTest(unittest.TestCase):
    password = None

    def setUp(self):
        self.values = {}
        self.sent = []
        obs.plugin._send = self.capture
        obs.plugin.watching = set()
        obs.live.published.clear()
        obs.live.publish_all()
        self.server = FakeObs(password=self.password)
        self.addCleanup(self.server.stop)

    def capture(self, message):
        self.sent.append(message)
        if message.get("event") == "value":
            self.values[message["id"]] = message["value"]

    def connect(self, server=None, password=None):
        server = server or self.server
        obs.configure({"url": server.url, "password": password or ""})
        wait_for(lambda: obs.client.connected and obs.client.url == server.url, message="connection")
        self.settle(server)

    def settle(self, server=None):
        server = server or self.server
        wait_for(lambda: any(t == "GetVersion" for t, _ in server.batched), message="initial sync")
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0, message="background jobs")

    def watch(self, *values):
        obs.plugin.watching = set(values)
        obs.watching(sorted(values))
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0, message="watch refresh")

    def value(self, key, expected, timeout=2.0):
        wait_for(lambda: self.values.get(key) == expected, timeout, f"{key} == {expected!r} (is {self.values.get(key)!r})")


class AuthAndFramingTest(unittest.TestCase):
    def test_auth_string_matches_the_protocol_docs_example(self):
        self.assertEqual(obs.auth_string("supersecretpassword", SALT, CHALLENGE), "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=")

    def test_frame_lengths(self):
        for n, header in ((10, 2), (125, 2), (126, 4), (65535, 4), (65536, 10), (70000, 10)):
            frame = obs.encode_frame(obs.OP_TEXT, b"x" * n)
            self.assertEqual(len(frame), header + 4 + n, n)  # + mask key
            self.assertTrue(frame[1] & 0x80, "client frames are masked")

    def test_masking_round_trip(self):
        key = b"\x01\x02\x03\x04"
        data = os.urandom(1001)
        self.assertEqual(obs.mask_bytes(obs.mask_bytes(data, key), key), data)
        self.assertEqual(obs.mask_bytes(b"", key), b"")

    def test_parse_url(self):
        self.assertEqual(obs.parse_url(""), (False, "127.0.0.1", 4455, "/"))
        self.assertEqual(obs.parse_url("192.168.1.20"), (False, "192.168.1.20", 4455, "/"))
        self.assertEqual(obs.parse_url("ws://studio.lan:4444"), (False, "studio.lan", 4444, "/"))
        self.assertEqual(obs.parse_url("wss://obs.example.org/ws"), (True, "obs.example.org", 443, "/ws"))
        with self.assertRaisesRegex(ValueError, "ws://"):
            obs.parse_url("http://127.0.0.1:4455")

    def test_volume_mapping_round_trip(self):
        for v in range(101):
            self.assertEqual(obs.mul_to_slider(obs.slider_to_mul(v)), v)
        self.assertEqual(obs.slider_to_mul(50), 0.25)
        self.assertEqual(obs.slider_to_mul(250), 1.0)
        self.assertEqual(obs.slider_to_mul(-5), 0.0)
        self.assertEqual(obs.mul_to_slider(3.0), 100)

    def test_split_pair(self):
        self.assertEqual(obs.split_pair("Gaming/Webcam"), ("Gaming", "Webcam"))
        self.assertEqual(obs.split_pair("Gaming / Web/cam"), ("Gaming", "Web/cam"))
        self.assertEqual(obs.split_pair("A/B|Cam"), ("A/B", "Cam"))

    def test_manifest_lists_every_value(self):
        here = os.path.dirname(os.path.abspath(__file__))
        with open(os.path.join(here, "perritodeck-plugin.json"), encoding="utf-8") as f:
            manifest = json.load(f)
        ids = {v["id"] for v in manifest["values"]}
        self.assertEqual(ids, set(obs.FIXED_VALUES) | set(obs.ARG_VALUES) | {"last_response"})
        for v in manifest["values"]:
            self.assertEqual("arg" in v, v["id"] in obs.ARG_VALUES, v["id"])
        self.assertEqual({a["id"] for a in manifest["actions"]}, set(obs.plugin._actions))
        self.assertEqual(manifest["id"], "dev.perritodeck.obs")
        password = [s for s in manifest["settings"] if s["id"] == "password"][0]
        self.assertEqual(password["type"], "password")


class ConnectionTest(PluginTest):
    def test_handshake_identify_and_initial_values(self):
        self.connect()
        line, headers = self.server.handshakes[-1]
        self.assertEqual(line, "GET / HTTP/1.1")
        self.assertEqual(headers["sec-websocket-protocol"], "obswebsocket.json")
        self.assertEqual(headers["sec-websocket-version"], "13")
        identify = self.server.identifies[-1]
        self.assertEqual(identify["op"], 1)
        self.assertEqual(identify["d"]["rpcVersion"], 1)
        self.assertNotIn("authentication", identify["d"])
        mask = identify["d"]["eventSubscriptions"]
        self.assertEqual(mask, 0x5FF)  # all low-volume categories but Vendors and Canvases
        self.assertFalse(mask & (1 << 16), "no high-volume volume meters")
        self.value("connected", "on")
        self.value("scene", "Live")
        self.value("studio_mode", "off")
        self.value("streaming", "off")
        self.value("replay_buffer", "off")
        self.value("transition", "Cut")
        self.value("profile", "Untitled")
        self.value("scene_collection", "Main")
        self.value("preview_scene", obs.UNKNOWN)
        self.assertEqual(self.server.unmasked, 0)
        self.assertEqual(self.server.requests, [], "the initial sync is one batch")

    def test_not_running(self):
        port = self.server.port
        self.server.stop()
        obs.configure({"url": f"ws://127.0.0.1:{port}"})
        wait_for(lambda: "refused" in obs.client.last_error, message="refused error")
        with self.assertRaisesRegex(RuntimeError, r"couldn't reach OBS at ws://127\.0\.0\.1:\d+: connection refused"):
            obs.stream({"mode": "start"})
        self.value("connected", "off")

    def test_reconnects_after_obs_crashes_and_restarts(self):
        self.connect()
        self.server.stop()  # OBS quits: no listener, connections gone
        self.value("connected", "off")
        self.value("scene", obs.UNKNOWN)
        restarted = FakeObs(port=self.server.port)
        self.addCleanup(restarted.stop)
        restarted.state["program"] = "After restart"
        wait_for(lambda: obs.client.connected, 3.0, "reconnection")
        self.settle(restarted)
        self.value("connected", "on")
        self.value("scene", "After restart")

    def test_reconnects_after_a_dropped_connection(self):
        self.connect()
        self.server.drop_all()
        wait_for(lambda: self.server.connections >= 2 and obs.client.connected, 3.0, "reconnection")
        obs.stream({"mode": "start"})
        self.assertIn(("StartStream", None), self.server.requests)

    def test_settings_change_reconnects_to_the_new_address(self):
        self.connect()
        other = FakeObs()
        self.addCleanup(other.stop)
        other.state["program"] = "Other"
        self.connect(other)
        self.value("scene", "Other")
        wait_for(lambda: not self.server.clients, message="old connection closed")
        obs.configure({"url": other.url})  # same settings: no reconnect
        time.sleep(0.05)
        self.assertEqual(other.connections, 1)

    def test_kicked_session_waits_for_an_action(self):
        self.connect()
        self.server.kick(4011)
        self.value("connected", "off")
        time.sleep(0.3)
        self.assertEqual(self.server.connections, 1, "must not reconnect by itself after a kick")
        self.assertIn("press a key", obs.client.last_error)
        obs.stream({"mode": "toggle"})  # a key press reconnects
        self.assertEqual(self.server.connections, 2)
        self.assertIn(("ToggleStream", None), self.server.requests)

    def test_server_ping_gets_a_pong(self):
        self.connect()
        self.server.clients[0].send(server_frame(obs.OP_PING, b"are you there"))
        wait_for(lambda: b"are you there" in self.server.pongs, message="pong")

    def test_request_timeout(self):
        self.connect()
        self.server.handlers["GetVersion"] = lambda d: time.sleep(1.5) or {}
        with self.assertRaises(TimeoutError):
            obs.client.call("GetVersion", timeout=0.2)


class AuthTest(PluginTest):
    password = "supersecretpassword"

    def test_right_password(self):
        self.connect(password="supersecretpassword")
        self.assertIn("authentication", self.server.identifies[-1]["d"])
        self.value("connected", "on")

    def test_wrong_password(self):
        obs.configure({"url": self.server.url, "password": "nope"})
        wait_for(lambda: "rejected the password" in obs.client.last_error, message="auth error")
        with self.assertRaisesRegex(RuntimeError, "OBS rejected the password"):
            obs.record({"mode": "start"})
        self.value("connected", "off")

    def test_missing_password(self):
        obs.configure({"url": self.server.url, "password": ""})
        wait_for(lambda: "needs a password" in obs.client.last_error, message="password needed")


class ActionTest(PluginTest):
    def setUp(self):
        super().setUp()
        self.connect()
        self.server.requests.clear()

    def run_action(self, fn, settings):
        self.server.requests.clear()
        return fn(settings)

    def test_actions_send_the_right_requests(self):
        mic = {"inputName": "Mic"}
        cases = [
            (obs.switch_scene, {"scene": "Gaming"}, [("SetCurrentProgramScene", {"sceneName": "Gaming"})]),
            (obs.switch_scene, {"scene": "Gaming", "where": "preview"}, [("SetCurrentPreviewScene", {"sceneName": "Gaming"})]),
            (obs.studio_mode, {"mode": "toggle"}, [("SetStudioModeEnabled", {"studioModeEnabled": True})]),
            (obs.studio_mode, {"mode": "off"}, [("SetStudioModeEnabled", {"studioModeEnabled": False})]),
            (obs.set_transition, {"transition": "Fade", "duration": "300"},
             [("SetCurrentSceneTransition", {"transitionName": "Fade"}), ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300})]),
            (obs.stream, {"mode": "toggle"}, [("ToggleStream", None)]),
            (obs.stream, {"mode": "start"}, [("StartStream", None)]),
            (obs.stream, {"mode": "stop"}, [("StopStream", None)]),
            (obs.record, {}, [("ToggleRecord", None)]),
            (obs.record, {"mode": "pause-toggle"}, [("ToggleRecordPause", None)]),
            (obs.record, {"mode": "pause"}, [("PauseRecord", None)]),
            (obs.record, {"mode": "resume"}, [("ResumeRecord", None)]),
            (obs.record_split, {}, [("SplitRecordFile", None)]),
            (obs.record_chapter, {"name": "Intro"}, [("CreateRecordChapter", {"chapterName": "Intro"})]),
            (obs.record_chapter, {}, [("CreateRecordChapter", {})]),
            (obs.replay_buffer, {"mode": "save"}, [("SaveReplayBuffer", None)]),
            (obs.replay_buffer, {"mode": "start"}, [("StartReplayBuffer", None)]),
            (obs.virtualcam, {"mode": "toggle"}, [("ToggleVirtualCam", None)]),
            (obs.virtualcam, {"mode": "stop"}, [("StopVirtualCam", None)]),
            (obs.mute, {"input": "Mic"}, [("ToggleInputMute", mic)]),
            (obs.mute, {"input": "Mic", "mode": "mute"}, [("SetInputMute", {"inputName": "Mic", "inputMuted": True})]),
            (obs.mute, {"input": "Mic", "mode": "unmute"}, [("SetInputMute", {"inputName": "Mic", "inputMuted": False})]),
            (obs.volume, {"input": "Mic", "value": 50}, [("SetInputVolume", {"inputName": "Mic", "inputVolumeMul": 0.25})]),
            (obs.volume, {"input": "Mic", "value": 100}, [("SetInputVolume", {"inputName": "Mic", "inputVolumeMul": 1.0})]),
            (obs.hotkey, {"hotkey": "OBSBasic.Screenshot"}, [("TriggerHotkeyByName", {"hotkeyName": "OBSBasic.Screenshot"})]),
            (obs.media, {"input": "Clip", "command": "restart"},
             [("TriggerMediaInputAction", {"inputName": "Clip", "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_RESTART"})]),
            (obs.media, {"input": "Clip", "command": "next"},
             [("TriggerMediaInputAction", {"inputName": "Clip", "mediaAction": "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_NEXT"})]),
            (obs.browser_refresh, {"input": "Chat"}, [("PressInputPropertiesButton", {"inputName": "Chat", "propertyName": "refreshnocache"})]),
            (obs.set_text, {"input": "Title", "text": "Back soon\\nreally"},
             [("SetInputSettings", {"inputName": "Title", "inputSettings": {"text": "Back soon\nreally"}})]),
            (obs.projector, {"what": "program", "monitor": "2"},
             [("OpenVideoMixProjector", {"monitorIndex": 1, "videoMixType": "OBS_WEBSOCKET_VIDEO_MIX_TYPE_PROGRAM"})]),
            (obs.projector, {"what": "multiview"}, [("OpenVideoMixProjector", {"videoMixType": "OBS_WEBSOCKET_VIDEO_MIX_TYPE_MULTIVIEW"})]),
            (obs.projector, {"what": "source", "source": "Webcam"}, [("OpenSourceProjector", {"sourceName": "Webcam"})]),
            (obs.profile, {"profile": "Streaming"}, [("SetCurrentProfile", {"profileName": "Streaming"})]),
            (obs.scene_collection, {"collection": "Podcast"}, [("SetCurrentSceneCollection", {"sceneCollectionName": "Podcast"})]),
            (obs.caption, {"text": "Hello"}, [("SendStreamCaption", {"captionText": "Hello"})]),
            (obs.send_request, {"request": "GetVersion"}, [("GetVersion", None)]),
            (obs.send_request, {"request": "SetCurrentProgramScene", "data": '{"sceneName": "A"}'},
             [("SetCurrentProgramScene", {"sceneName": "A"})]),
            (obs.send_request_value, {"request": "SetInputVolume", "data": '{"inputName": "Mic", "inputVolumeDb": {value}}', "value": 37},
             [("SetInputVolume", {"inputName": "Mic", "inputVolumeDb": 37})]),
            (obs.send_request_value, {"request": "SetInputVolume", "data": '{"inputName": "Mic", "inputVolumeMul": {mul}}', "value": 50},
             [("SetInputVolume", {"inputName": "Mic", "inputVolumeMul": 0.25})]),
        ]
        for fn, settings, expected in cases:
            with self.subTest(action=fn.__name__, settings=settings):
                self.assertIsNone(self.run_action(fn, settings))
                self.assertEqual(self.server.requests, expected)

    def test_scene_goes_to_the_preview_in_studio_mode(self):
        self.server.event("StudioModeStateChanged", {"studioModeEnabled": True})
        self.value("studio_mode", "on")
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0)
        self.run_action(obs.switch_scene, {"scene": "Gaming"})
        self.assertEqual(self.server.requests, [("SetCurrentPreviewScene", {"sceneName": "Gaming"})])
        self.run_action(obs.switch_scene, {"scene": "Gaming", "where": "program"})
        self.assertEqual(self.server.requests, [("SetCurrentProgramScene", {"sceneName": "Gaming"})])

    def test_transition_with_override_puts_the_old_one_back(self):
        self.server.handlers["GetCurrentSceneTransition"] = lambda d: {
            "transitionName": "Cut", "transitionFixed": False, "transitionDuration": 300}
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Fade", "duration": 500}))
        self.assertEqual(self.server.requests, [
            ("GetCurrentSceneTransition", None),
            ("SetCurrentSceneTransition", {"transitionName": "Fade"}),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 500}),
            ("TriggerStudioModeTransition", None),
        ])
        self.server.event("SceneTransitionEnded", {"transitionName": "Fade"})
        wait_for(lambda: len(self.server.requests) == 6, message="restore")
        self.assertEqual(self.server.requests[4:], [
            ("SetCurrentSceneTransition", {"transitionName": "Cut"}),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300}),
        ])

    def test_transition_outside_studio_mode_warns(self):
        def refuse(d):
            raise Fail(506)
        self.server.handlers["TriggerStudioModeTransition"] = refuse
        self.assertIn("Studio Mode is off", self.run_action(obs.transition_now, {}))
        self.assertEqual(self.server.requests, [("TriggerStudioModeTransition", None)])

    def test_output_warnings(self):
        def running(d):
            raise Fail(500, "The stream output is already active.")
        self.server.handlers["StartStream"] = running
        self.assertEqual(self.run_action(obs.stream, {"mode": "start"}), "OBS is already streaming")

        def not_running(d):
            raise Fail(501)
        self.server.handlers["StopRecord"] = not_running
        self.assertEqual(self.run_action(obs.record, {"mode": "stop"}), "OBS isn't recording")

        def disabled(d):
            raise Fail(504, "Replay buffer is not enabled.")
        self.server.handlers["SaveReplayBuffer"] = disabled
        with self.assertRaisesRegex(RuntimeError, "replay buffer is turned off"):
            self.run_action(obs.replay_buffer, {"mode": "save"})

    def test_request_failure_includes_obs_comment(self):
        with self.assertRaisesRegex(RuntimeError, "OBS: No source was found by the name of `Nope`."):
            self.run_action(obs.switch_scene, {"scene": "Nope"})

    def test_missing_fields(self):
        with self.assertRaisesRegex(ValueError, "scene name"):
            obs.switch_scene({})
        with self.assertRaisesRegex(ValueError, "audio source"):
            obs.mute({"mode": "mute"})
        with self.assertRaisesRegex(ValueError, "between 50 and 20000"):
            obs.set_transition({"transition": "Fade", "duration": 10})
        with self.assertRaisesRegex(ValueError, "valid JSON"):
            obs.send_request({"request": "GetVersion", "data": "{nope"})
        with self.assertRaisesRegex(ValueError, "JSON object"):
            obs.send_request({"request": "GetVersion", "data": "[1]"})
        self.assertEqual(self.server.requests, [])

    def test_split_and_chapter_need_a_newer_obs(self):
        obs.live.available = {"GetVersion"}
        self.assertIn("30.2", self.run_action(obs.record_split, {}))
        self.assertIn("30.2", self.run_action(obs.record_chapter, {"name": "x"}))
        self.assertEqual(self.server.requests, [], "nothing sent when OBS lacks the request")
        obs.live.available = None

        def unknown(d):
            raise Fail(204, "Your request type is not valid.")
        self.server.handlers["SplitRecordFile"] = unknown
        self.assertIn("30.2", self.run_action(obs.record_split, {}))

    def test_volume_step(self):
        self.server.handlers["GetInputVolume"] = lambda d: {"inputVolumeMul": 0.25, "inputVolumeDb": -12.04}
        self.run_action(obs.volume_step, {"input": "Mic", "step": 10})
        self.assertEqual(self.server.requests, [
            ("GetInputVolume", {"inputName": "Mic"}),
            ("SetInputVolume", {"inputName": "Mic", "inputVolumeMul": 0.36}),
        ])
        self.run_action(obs.volume_step, {"input": "Mic", "step": -80})
        self.assertEqual(self.server.requests[-1], ("SetInputVolume", {"inputName": "Mic", "inputVolumeMul": 0.0}))

    def test_source_visibility_toggle_uses_the_live_scene(self):
        self.server.handlers["GetSceneItemId"] = lambda d: {"sceneItemId": 7}
        self.server.handlers["GetSceneItemEnabled"] = lambda d: {"sceneItemEnabled": True}
        self.run_action(obs.source_visibility, {"source": "Cam"})
        self.assertEqual(self.server.requests, [
            ("GetSceneItemId", {"sceneName": "Live", "sourceName": "Cam"}),
            ("GetSceneItemEnabled", {"sceneName": "Live", "sceneItemId": 7}),
            ("SetSceneItemEnabled", {"sceneName": "Live", "sceneItemId": 7, "sceneItemEnabled": False}),
        ])
        self.run_action(obs.source_visibility, {"scene": "Other", "source": "Cam", "mode": "show"})
        self.assertEqual(self.server.requests[-1], ("SetSceneItemEnabled", {"sceneName": "Other", "sceneItemId": 7, "sceneItemEnabled": True}))

    def test_source_visibility_missing_source(self):
        def missing(d):
            raise Fail(600, "No scene items were found in the specified scene by that name or offset.")
        self.server.handlers["GetSceneItemId"] = missing
        with self.assertRaisesRegex(RuntimeError, "no source “Cam” in the scene “Live”"):
            self.run_action(obs.source_visibility, {"source": "Cam"})

    def test_filter_toggle(self):
        self.server.handlers["GetSourceFilter"] = lambda d: {"filterEnabled": False, "filterIndex": 0, "filterKind": "x", "filterSettings": {}}
        self.run_action(obs.source_filter, {"source": "Cam", "filter": "Blur"})
        self.assertEqual(self.server.requests, [
            ("GetSourceFilter", {"sourceName": "Cam", "filterName": "Blur"}),
            ("SetSourceFilterEnabled", {"sourceName": "Cam", "filterName": "Blur", "filterEnabled": True}),
        ])
        self.run_action(obs.source_filter, {"source": "Cam", "filter": "Blur", "mode": "off"})
        self.assertEqual(self.server.requests, [("SetSourceFilterEnabled", {"sourceName": "Cam", "filterName": "Blur", "filterEnabled": False})])

    def test_media_play_pause_follows_the_state(self):
        for state, action in (("OBS_MEDIA_STATE_PLAYING", "PAUSE"), ("OBS_MEDIA_STATE_PAUSED", "PLAY"), ("OBS_MEDIA_STATE_ENDED", "RESTART")):
            self.server.handlers["GetMediaInputStatus"] = lambda d, s=state: {"mediaState": s}
            self.run_action(obs.media, {"input": "Clip"})
            self.assertEqual(self.server.requests[-1][1]["mediaAction"], "OBS_WEBSOCKET_MEDIA_INPUT_ACTION_" + action)

    def test_screenshot(self):
        with tempfile.TemporaryDirectory() as folder:
            target = os.path.join(folder, "shots")
            self.run_action(obs.screenshot, {"folder": target, "format": "jpg"})
            self.assertTrue(os.path.isdir(target), "the folder is made when OBS is on this computer")
        (request_type, data), = self.server.requests
        self.assertEqual(request_type, "SaveSourceScreenshot")
        self.assertEqual(data["sourceName"], "Live")
        self.assertEqual(data["imageFormat"], "jpg")
        self.assertTrue(data["imageFilePath"].startswith(os.path.join(target, "Live ")))
        self.assertRegex(data["imageFilePath"], r"Live \d{4}-\d\d-\d\d \d\d-\d\d-\d\d\.jpg$")
        self.run_action(obs.screenshot, {"source": "Web:cam?", "folder": folder})
        self.assertIn("Web_cam_ ", self.server.requests[-1][1]["imageFilePath"])

    def test_default_screenshot_folder_is_pictures(self):
        folder = obs.pictures_folder()
        self.assertTrue(os.path.isabs(folder))

    def test_request_publishes_the_answer(self):
        self.run_action(obs.send_request, {"request": "GetProfileList"})
        self.value("last_response", '{"currentProfileName":"Untitled","profiles":["Untitled"]}')

    def test_large_messages_from_the_client(self):
        for size in (200, 70000):
            text = "x" * size
            self.run_action(obs.set_text, {"input": "Title", "text": text})
            self.assertEqual(len(self.server.requests[-1][1]["inputSettings"]["text"]), size)
        self.assertEqual(self.server.unmasked, 0)


class ValueTest(PluginTest):
    def setUp(self):
        super().setUp()
        self.connect()

    def test_scene_events(self):
        self.watch("scene_active:Live", "scene_active:Gaming", "scene_preview:Gaming")
        self.value("scene_active:Live", "on")
        self.value("scene_active:Gaming", "off")
        self.value("scene_preview:Gaming", "off")
        self.server.event("CurrentProgramSceneChanged", {"sceneName": "Gaming", "sceneUuid": "u"})
        self.value("scene", "Gaming")
        self.value("scene_active:Gaming", "on")
        self.value("scene_active:Live", "off")
        self.server.handlers["GetCurrentPreviewScene"] = lambda d: {"sceneName": "Gaming"}
        self.server.event("StudioModeStateChanged", {"studioModeEnabled": True})
        self.value("studio_mode", "on")
        self.value("preview_scene", "Gaming")
        self.value("scene_preview:Gaming", "on")
        self.server.event("CurrentPreviewSceneChanged", {"sceneName": "Live"})
        self.value("preview_scene", "Live")
        self.value("scene_preview:Gaming", "off")

    def test_output_events(self):
        self.server.event("StreamStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.value("streaming", "on")
        self.server.event("RecordStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED", "outputPath": None})
        self.value("recording", "on")
        self.value("record_paused", "off")
        self.server.event("RecordStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_PAUSED", "outputPath": None})
        self.value("record_paused", "on")
        self.server.event("RecordStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_RESUMED", "outputPath": None})
        self.value("record_paused", "off")
        self.server.event("ReplayBufferStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.value("replay_buffer", "on")
        self.server.event("VirtualcamStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.value("virtualcam", "on")
        self.server.event("StreamStateChanged", {"outputActive": False, "outputState": "OBS_WEBSOCKET_OUTPUT_STOPPED"})
        self.value("streaming", "off")
        self.value("stream_time", "00:00:00")
        self.server.event("CurrentSceneTransitionChanged", {"transitionName": "Fade"})
        self.value("transition", "Fade")
        self.server.event("CurrentProfileChanged", {"profileName": "Podcast"})
        self.value("profile", "Podcast")

    def test_audio_values_follow_events(self):
        self.server.handlers["GetInputMute"] = lambda d: {"inputMuted": False}
        self.server.handlers["GetInputVolume"] = lambda d: {"inputVolumeMul": 0.25, "inputVolumeDb": -12.0412}
        self.watch("muted:Mic", "volume:Mic", "volume_db:Mic")
        self.value("muted:Mic", "off")
        self.value("volume:Mic", "50")
        self.value("volume_db:Mic", "-12")
        self.server.event("InputMuteStateChanged", {"inputName": "Mic", "inputUuid": "u", "inputMuted": True})
        self.value("muted:Mic", "on")
        self.server.event("InputVolumeChanged", {"inputName": "Mic", "inputUuid": "u", "inputVolumeMul": 0.81, "inputVolumeDb": -1.83})
        self.value("volume:Mic", "90")
        self.value("volume_db:Mic", "-1.8")
        self.server.event("InputVolumeChanged", {"inputName": "Mic", "inputUuid": "u", "inputVolumeMul": 0, "inputVolumeDb": -100})
        self.value("volume_db:Mic", "-inf")

    def test_only_watched_values_are_fetched(self):
        self.server.handlers["GetInputMute"] = lambda d: {"inputMuted": True}
        self.assertNotIn("GetInputMute", [t for t, _ in self.server.batched])
        self.watch("muted:Mic")
        self.value("muted:Mic", "on")
        self.assertEqual([d for t, d in self.server.batched if t == "GetInputMute"], [{"inputName": "Mic"}])
        self.watch("muted:Mic", "muted:Desktop Audio")  # only the new one is fetched
        self.assertEqual([d["inputName"] for t, d in self.server.batched if t == "GetInputMute"], ["Mic", "Desktop Audio"])
        self.assertNotIn("muted:Other", self.values)

    def test_scene_item_visibility(self):
        ids = {("Live", "Cam"): 7, ("Live", "Over/lay"): 9}

        def item_id(d):
            if (d["sceneName"], d["sourceName"]) not in ids:
                raise Fail(600, "No scene items were found")
            return {"sceneItemId": ids[(d["sceneName"], d["sourceName"])]}
        self.server.handlers["GetSceneItemId"] = item_id
        self.server.handlers["GetSceneItemEnabled"] = lambda d: {"sceneItemEnabled": d["sceneItemId"] == 7}
        self.watch("item_visible:Live/Cam", "item_visible:Live|Over/lay", "item_visible:Live/Missing")
        self.value("item_visible:Live/Cam", "on")
        self.value("item_visible:Live|Over/lay", "off")
        self.value("item_visible:Live/Missing", obs.UNKNOWN)
        self.server.event("SceneItemEnableStateChanged", {"sceneName": "Live", "sceneUuid": "u", "sceneItemId": 7, "sceneItemEnabled": False})
        self.value("item_visible:Live/Cam", "off")
        self.server.event("SceneItemEnableStateChanged", {"sceneName": "Other", "sceneUuid": "u", "sceneItemId": 9, "sceneItemEnabled": True})
        time.sleep(0.05)
        self.assertEqual(self.values["item_visible:Live|Over/lay"], "off", "different scene, same id")
        # The missing item appears: OBS says a scene item was created, so we look again.
        ids[("Live", "Missing")] = 11
        self.server.event("SceneItemCreated", {"sceneName": "Live", "sourceName": "Missing", "sceneItemId": 11, "sceneItemIndex": 2})
        self.value("item_visible:Live/Missing", "off")

    def test_renamed_input_is_looked_up_again(self):
        names = {"Mic": False}

        def get_mute(d):
            if d["inputName"] not in names:
                raise Fail(600, "No source was found by the name of `%s`." % d["inputName"])
            return {"inputMuted": names[d["inputName"]]}
        self.server.handlers["GetInputMute"] = get_mute
        self.watch("muted:Headset")
        self.value("muted:Headset", obs.UNKNOWN)
        names["Headset"] = True
        self.server.event("InputNameChanged", {"inputUuid": "u", "oldInputName": "Mic", "inputName": "Headset"})
        self.value("muted:Headset", "on")

    def test_filter_values(self):
        self.server.handlers["GetSourceFilter"] = lambda d: {"filterEnabled": True}
        self.watch("filter_enabled:Cam/Blur")
        self.value("filter_enabled:Cam/Blur", "on")
        self.server.event("SourceFilterEnableStateChanged", {"sourceName": "Cam", "filterName": "Blur", "filterEnabled": False})
        self.value("filter_enabled:Cam/Blur", "off")

    def test_text_values_and_large_or_fragmented_frames(self):
        self.server.handlers["GetInputSettings"] = lambda d: {"inputSettings": {"text": "hello"}, "inputKind": "text_ft2_source_v2"}
        self.watch("input_text:Title")
        self.value("input_text:Title", "hello")
        for size in (200, 70000):  # 16-bit and 64-bit frame lengths
            self.server.event("InputSettingsChanged", {"inputName": "Title", "inputUuid": "u", "inputSettings": {"text": "y" * size}})
            self.value("input_text:Title", "y" * size)
        # A message split over three frames, with a ping in the middle.
        conn = self.server.clients[0]
        data = json.dumps({"op": 5, "d": {"eventType": "InputSettingsChanged", "eventIntent": 8,
                                          "eventData": {"inputName": "Title", "inputSettings": {"text": "fragmented ✓"}}}}).encode()
        third = len(data) // 3
        conn.send(server_frame(obs.OP_TEXT, data[:third], fin=False)
                  + server_frame(obs.OP_PING, b"mid")
                  + server_frame(obs.OP_CONT, data[third:2 * third], fin=False)
                  + server_frame(obs.OP_CONT, data[2 * third:], fin=True))
        self.value("input_text:Title", "fragmented ✓")
        wait_for(lambda: b"mid" in self.server.pongs, message="pong inside a fragmented message")

    def test_media_values(self):
        self.server.handlers["GetMediaInputStatus"] = lambda d: {"mediaState": "OBS_MEDIA_STATE_PAUSED"}
        self.watch("media_playing:Clip")
        self.value("media_playing:Clip", "off")
        self.server.event("MediaInputPlaybackStarted", {"inputName": "Clip", "inputUuid": "u"})
        self.value("media_playing:Clip", "on")
        self.server.event("MediaInputPlaybackEnded", {"inputName": "Clip", "inputUuid": "u"})
        self.value("media_playing:Clip", "off")

    def test_disconnect_publishes_unknown(self):
        self.server.handlers["GetInputMute"] = lambda d: {"inputMuted": True}
        self.watch("muted:Mic")
        self.value("muted:Mic", "on")
        self.server.stop()
        self.value("connected", "off")
        self.value("muted:Mic", obs.UNKNOWN)
        self.value("streaming", obs.UNKNOWN)

    def test_stats_polling(self):
        clock = [100.0]
        obs.live.clock = lambda: clock[0]
        self.addCleanup(setattr, obs.live, "clock", time.monotonic)
        self.assertNotIn("GetStats", [t for t, _ in self.server.batched], "not polled until the timer runs")
        obs.poll()
        self.value("fps", "60")
        self.value("cpu", "3.1")
        self.value("memory", "513")
        self.value("dropped_frames", "0.3")  # (1 + 2) / (900 + 100)
        self.value("stream_time", "00:00:00")
        self.value("bitrate", "0")
        self.server.state["streaming"] = True
        self.server.state["recording"] = True
        self.server.state["stream_bytes"] = 1_000_000
        obs.poll()
        self.value("streaming", "on")
        self.value("stream_time", "00:01:02")
        self.value("record_time", "01:00:00")
        clock[0] += 2.0
        self.server.state["stream_bytes"] = 1_000_000 + 1_500_000  # 1.5 MB in 2 s = 6000 kbps
        obs.poll()
        self.value("bitrate", "6000")
        stats = [t for t, _ in self.server.batched if t == "GetStats"]
        self.assertEqual(len(stats), 3)

    def test_polling_does_nothing_when_disconnected(self):
        self.server.stop()
        self.value("connected", "off")
        before = len(self.server.batched)
        obs.poll()
        self.assertEqual(len(self.server.batched), before)

    def test_scene_collection_change_resyncs(self):
        self.server.event("CurrentSceneCollectionChanging", {"sceneCollectionName": "Main"})
        wait_for(lambda: obs.live.changing)
        self.server.state["program"] = "Podcast scene"
        self.server.handlers["GetSceneCollectionList"] = lambda d: {"currentSceneCollectionName": "Podcast"}
        self.server.event("CurrentSceneCollectionChanged", {"sceneCollectionName": "Podcast"})
        self.value("scene_collection", "Podcast")
        self.value("scene", "Podcast scene")


class RawWebSocketTest(unittest.TestCase):
    def test_fragmented_client_message(self):
        server = FakeObs()
        self.addCleanup(server.stop)
        ws = obs.WebSocket.connect(server.url)
        self.addCleanup(ws.abort)
        ws.settimeout(2)
        self.assertEqual(ws.protocol, "obswebsocket.json")
        self.assertEqual(json.loads(ws.recv())["op"], 0)
        ws.send_text(json.dumps({"op": 1, "d": {"rpcVersion": 1}}))
        self.assertEqual(json.loads(ws.recv())["op"], 2)
        request = {"op": 6, "d": {"requestType": "SetCurrentProgramScene", "requestId": "r1",
                                  "requestData": {"sceneName": "S" * 300}}}
        ws.send_text(json.dumps(request), fragment=50)
        answer = json.loads(ws.recv())
        self.assertEqual((answer["op"], answer["d"]["requestId"]), (7, "r1"))
        self.assertEqual(server.requests, [("SetCurrentProgramScene", {"sceneName": "S" * 300})])

    def test_not_a_websocket_server(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        self.addCleanup(listener.close)

        def answer():
            conn, _ = listener.accept()
            conn.recv(4096)
            conn.sendall(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            conn.close()
        threading.Thread(target=answer, daemon=True).start()
        with self.assertRaisesRegex(ConnectionError, "isn't a WebSocket server"):
            obs.WebSocket.connect(f"ws://127.0.0.1:{listener.getsockname()[1]}")


if __name__ == "__main__":
    unittest.main()
