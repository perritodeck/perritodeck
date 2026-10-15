"""Tests against a fake obs-websocket 5 server (RFC 6455 server side + op codes)."""

import base64
import hashlib
import json
import os
import queue
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
obs.OUTPUT_EVENT_WAIT = 0.4
obs.STREAM_START_WAIT = 1.0
obs.SWITCH_WAIT = 0.4
# configure() doesn't interrupt a connection attempt that is under way, and a test's
# configure() can land in one aimed at the previous test's stopped server. Linux refuses
# that at once; Windows retries the SYN for about 2 s first. So waits that follow
# configure() allow for one whole attempt (CONNECT_TIMEOUT at most) before their own.
AFTER_CONFIGURE = obs.CONNECT_TIMEOUT + 2.0

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
            "stream_bytes": 0, "stream_skipped": 0, "available": list(ALL_REQUESTS),
            "scenes": ["Live"], "groups": [], "paused": False,
            # Real OBS can pause only a recording with an encoder of its own, and a stream to a
            # server that isn't there stops about 2 s after starting (0.1 s here).
            "can_pause": True, "stream_fails": False, "stop_takes": 0.05,
            # One duration shared by every transition; None for the ones with a fixed length.
            "transition": "Cut", "duration": 300,
            "transitions": {"Cut": True, "Fade": False, "Swipe": False, "Stinger": True},
            "monitors": 2,
        }
        self.handlers = {}
        self.running = True
        self.steps = queue.Queue()
        threading.Thread(target=self._accept, daemon=True).start()
        threading.Thread(target=self._run_steps, daemon=True).start()

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
            # obs-websocket 5.5.6's Hello: no OBS version in it (GetVersion has that).
            hello = {"obsWebSocketVersion": "5.5.2", "rpcVersion": 1}
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
                "scenes": [{"sceneName": name, "sceneIndex": i} for i, name in enumerate(self.state["scenes"])]}

    def on_GetGroupList(self, d):
        return {"groups": list(self.state["groups"])}

    def on_GetStudioModeEnabled(self, d):
        return {"studioModeEnabled": self.state["studio"]}

    def on_GetStreamStatus(self, d):
        active = self.state["streaming"]
        return {"outputActive": active, "outputReconnecting": False,
                "outputTimecode": "00:01:02.345" if active else "00:00:00.000", "outputDuration": 62345,
                "outputCongestion": 0, "outputBytes": self.state["stream_bytes"],
                "outputSkippedFrames": self.state["stream_skipped"], "outputTotalFrames": 400 if active else 0}

    def on_GetRecordStatus(self, d):
        active = self.state["recording"]
        return {"outputActive": active, "outputPaused": active and self.state["paused"],
                "outputTimecode": "01:00:00.000" if active else "00:00:00.000", "outputDuration": 0, "outputBytes": 0}

    def on_GetReplayBufferStatus(self, d):
        raise Fail(604, "Replay buffer is not available.")

    def on_GetVirtualCamStatus(self, d):
        return {"outputActive": False}

    def on_GetCurrentSceneTransition(self, d):
        # obs-websocket reports no duration for a transition with a fixed length (Cut, Stinger).
        name = self.state["transition"]
        fixed = self.state["transitions"][name]
        return {"transitionName": name, "transitionKind": name.lower() + "_transition", "transitionFixed": fixed,
                "transitionDuration": None if fixed else self.state["duration"],
                "transitionConfigurable": not fixed, "transitionSettings": None}

    def on_GetSceneTransitionList(self, d):
        return {"currentSceneTransitionName": self.state["transition"], "transitions": [
            {"transitionName": name, "transitionKind": name.lower() + "_transition", "transitionFixed": fixed,
             "transitionConfigurable": not fixed}
            for name, fixed in self.state["transitions"].items()]}

    def on_SetCurrentSceneTransition(self, d):
        name = d.get("transitionName")
        if name not in self.state["transitions"]:
            raise Fail(600, "No scene transition was found by that name.")
        # OBS answers first and switches a moment later, in its window, then says so.
        self.later(self.then(0.005, "CurrentSceneTransitionChanged", {"transitionName": name}, transition=name))

    def on_SetCurrentSceneTransitionDuration(self, d):
        # One duration for every transition: OBS takes it even while Cut is current.
        duration = d["transitionDuration"]
        self.later(self.then(0.005, "CurrentSceneTransitionDurationChanged", {"transitionDuration": duration},
                             duration=duration))

    def on_TriggerStudioModeTransition(self, d):
        if not self.state["studio"]:
            raise Fail(506)
        name = self.state["transition"]
        self.later(self.then(0.005, "SceneTransitionStarted", {"transitionName": name}),
                   self.then(0.01, "SceneTransitionEnded", {"transitionName": name}))

    def on_GetMonitorList(self, d):
        return {"monitors": [{"monitorName": f"Monitor {i + 1}", "monitorIndex": i, "monitorWidth": 1920,
                              "monitorHeight": 1080, "monitorPositionX": 1920 * i, "monitorPositionY": 0}
                             for i in range(self.state["monitors"])]}

    # Outputs, the way OBS 31 runs them: it answers at once and starts or stops a moment later,
    # with StreamStateChanged / RecordStateChanged events whose outputActive is true only for
    # STARTED and RESUMED. While a recording stops, GetRecordStatus still says active.

    def output_event(self, event_type, state, active, delay=0.005, **changes):
        data = {"outputActive": active, "outputState": "OBS_WEBSOCKET_OUTPUT_" + state}
        if event_type == "RecordStateChanged":
            data["outputPath"] = None
        return self.then(delay, event_type, data, **changes)

    def on_StartStream(self, d):
        if self.state["streaming"]:
            raise Fail(500)
        self._start_stream()

    def on_StopStream(self, d):
        if not self.state["streaming"]:
            raise Fail(501)
        self._stop_stream()

    def on_ToggleStream(self, d):
        if self.state["streaming"]:
            self._stop_stream()
        else:
            self._start_stream()
        return {"outputActive": not self.state["streaming"]}

    def _start_stream(self):
        if self.state["stream_fails"]:
            self.later(self.output_event("StreamStateChanged", "STARTING", False),
                       self.output_event("StreamStateChanged", "STOPPED", False, delay=0.1))
        else:
            self.later(self.output_event("StreamStateChanged", "STARTING", False),
                       self.output_event("StreamStateChanged", "STARTED", True, streaming=True))

    def _stop_stream(self):
        self.later(self.output_event("StreamStateChanged", "STOPPING", False),
                   self.output_event("StreamStateChanged", "STOPPED", False, delay=self.state["stop_takes"],
                                     streaming=False))

    def on_StartRecord(self, d):
        if self.state["recording"]:
            raise Fail(500)
        self._start_record()

    def on_StopRecord(self, d):
        if not self.state["recording"]:
            raise Fail(501)
        self._stop_record()
        return {"outputPath": "/videos/recording.mp4"}

    def on_ToggleRecord(self, d):
        active = not self.state["recording"]
        if active:
            self._start_record()
        else:
            self._stop_record()
        return {"outputActive": active}

    def _start_record(self):
        self.state["recording"] = True
        self.later(self.output_event("RecordStateChanged", "STARTING", False),
                   self.output_event("RecordStateChanged", "STARTED", True))

    def _stop_record(self):
        self.later(self.output_event("RecordStateChanged", "STOPPING", False),
                   self.output_event("RecordStateChanged", "STOPPED", False, delay=self.state["stop_takes"],
                                     recording=False, paused=False))

    def on_PauseRecord(self, d):
        if self.state["paused"]:
            raise Fail(502)
        self._pause_record()

    def on_ResumeRecord(self, d):
        if not self.state["paused"]:
            raise Fail(503)
        self._resume_record()

    def on_ToggleRecordPause(self, d):
        if self.state["paused"]:
            self._resume_record()
        else:
            self._pause_record()

    def _pause_record(self):
        # Yes, even with nothing recording or a recording that can't pause; then nothing happens.
        if self.state["recording"] and self.state["can_pause"]:
            self.state["paused"] = True
            self.later(self.output_event("RecordStateChanged", "PAUSED", False))

    def _resume_record(self):
        self.state["paused"] = False
        self.later(self.output_event("RecordStateChanged", "RESUMED", True))

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

    def then(self, delay, event_type, data, **changes):
        """A step for later(): after `delay`, change the state and send the event."""
        def step():
            time.sleep(delay)
            self.state.update(changes)
            self.event(event_type, data)
        return step

    def later(self, *steps):
        """Run steps after the answer has gone, one after another, as OBS's window does with
        the work obs-websocket hands it."""
        for step in steps:
            self.steps.put(step)

    def _run_steps(self):
        while True:
            step = self.steps.get()
            if step is None:
                return
            try:
                step()
            except OSError:
                pass

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
        self.steps.put(None)
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
        wait_for(lambda: obs.client.connected and obs.client.url == server.url, AFTER_CONFIGURE, "connection")
        self.settle(server)

    def settle(self, server=None):
        server = server or self.server
        wait_for(lambda: any(t == "GetVersion" for t, _ in server.batched), message="initial sync")
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0, message="background jobs")

    def watch(self, *values):
        obs.plugin.watching = set(values)
        obs.watching(sorted(values))
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0, message="watch refresh")

    def resync(self):
        """OBS switches to the same scene collection again: the plugin reads everything again."""
        syncs = [t for t, _ in self.server.batched].count("GetVersion")
        self.server.event("CurrentSceneCollectionChanging", {"sceneCollectionName": "Main"})
        self.server.event("CurrentSceneCollectionChanged", {"sceneCollectionName": "Main"})
        wait_for(lambda: [t for t, _ in self.server.batched].count("GetVersion") > syncs, message="the next sync")
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0, message="background jobs")

    def value(self, key, expected, timeout=2.0):
        wait_for(lambda: self.values.get(key) == expected, timeout, f"{key} == {expected!r} (is {self.values.get(key)!r})")

    def published(self, key):
        """Every value published for `key` so far, in order."""
        return [m["value"] for m in list(self.sent) if m.get("event") == "value" and m.get("id") == key]

    def logs(self):
        return [m["message"] for m in list(self.sent) if m.get("event") == "log"]


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

    def test_splits_tries_the_first_bar_then_the_first_slash_then_the_rest(self):
        self.assertEqual(obs.splits("Gaming/Webcam"), [("Gaming", "Webcam")])
        self.assertEqual(obs.splits("Gaming / Web/cam"), [("Gaming", "Web/cam"), ("Gaming / Web", "cam")])
        self.assertEqual(obs.splits("A/B|Cam"), [("A/B", "Cam"), ("A", "B|Cam")])
        # The README's rule: the first '/' splits, also when the second name has a '|'.
        self.assertIn(("PD BRB", "PD Odd: A|B"), obs.splits("PD BRB/PD Odd: A|B"))
        self.assertEqual(obs.splits("A|B/Cam|x"), [("A", "B/Cam|x"), ("A|B", "Cam|x"), ("A|B/Cam", "x")])
        self.assertEqual(obs.splits("Webcam"), [])
        self.assertEqual(obs.splits("/Webcam"), [])
        self.assertEqual(obs.splits("Gaming/ "), [])

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


class FrameRulesTest(unittest.TestCase):
    """The client refuses frames a server must never send (RFC 6455 5.1, 5.4, 5.5)."""

    def received(self, *frames):
        a, b = socket.socketpair()
        self.addCleanup(a.close)
        self.addCleanup(b.close)
        ws = obs.WebSocket(a)
        b.sendall(b"".join(frames))
        return ws.recv()

    def refused(self, code, *frames):
        with self.assertRaises(obs.WebSocketClosed) as caught:
            self.received(*frames)
        self.assertEqual(caught.exception.code, code)

    def test_fragments_and_pings_between_them(self):
        message = self.received(
            server_frame(obs.OP_TEXT, b"he", fin=False),
            server_frame(obs.OP_PING, b"p"),
            server_frame(obs.OP_CONT, "llo ☕".encode(), fin=True),
        )
        self.assertEqual(message, "hello ☕")

    def test_masked_server_frames_are_refused(self):
        self.refused(1002, obs.encode_frame(obs.OP_TEXT, b"hi", mask=True))

    def test_control_frame_rules(self):
        self.refused(1002, server_frame(obs.OP_PING, b"x" * 126))
        self.refused(1002, server_frame(obs.OP_PING, b"x", fin=False))
        self.refused(1002, server_frame(0xB, b""))

    def test_fragment_ordering(self):
        self.refused(1002, server_frame(obs.OP_CONT, b"x"))
        self.refused(1002, server_frame(obs.OP_TEXT, b"a", fin=False), server_frame(obs.OP_TEXT, b"b"))
        self.refused(1002, bytes([0x80 | 0x40 | obs.OP_TEXT, 1]) + b"x")  # RSV1 without an extension

    def test_size_limit(self):
        self.assertLessEqual(obs.MAX_MESSAGE, 8 << 20)
        # A header announcing a too-big frame is refused before reading the payload.
        self.refused(1009, bytes([0x80 | obs.OP_TEXT, 127]) + struct.pack("!Q", obs.MAX_MESSAGE + 1))
        old = obs.MAX_MESSAGE
        obs.MAX_MESSAGE = 10
        try:
            # Fragments that add up to too much are refused too.
            self.refused(
                1009, server_frame(obs.OP_TEXT, b"x" * 6, fin=False), server_frame(obs.OP_CONT, b"x" * 6)
            )
        finally:
            obs.MAX_MESSAGE = old


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

    def test_connect_log_names_obs_and_obs_websocket_versions_once(self):
        self.connect()
        expected = f"connected to OBS 30.2.2 (obs-websocket 5.5.2) at {self.server.url}"
        wait_for(lambda: expected in self.logs(), message="the connect log line")
        # A scene collection change syncs again, but the connection is the same.
        self.resync()
        self.assertEqual([m for m in self.logs() if m.startswith("connected to")], [expected])

    def test_connect_log_without_get_version_names_the_hellos_obs_websocket_version(self):
        def not_ready(d):
            raise Fail(207, "OBS is not ready to perform the request.")
        self.server.handlers["GetVersion"] = not_ready
        self.connect()
        expected = f"connected to OBS ? (obs-websocket 5.5.2) at {self.server.url}"
        wait_for(lambda: expected in self.logs(), message="the connect log line")

    def test_statistics_come_with_the_first_sync(self):
        self.connect()
        self.value("fps", "60")
        self.value("cpu", "3.1")
        self.value("dropped_frames", "0.3")

    def test_not_running(self):
        port = self.server.port
        self.server.stop()
        obs.configure({"url": f"ws://127.0.0.1:{port}"})
        # Its own attempt is refused too, which also takes Windows about 2 s.
        wait_for(lambda: "refused" in obs.client.last_error, AFTER_CONFIGURE + obs.CONNECT_TIMEOUT, "refused error")
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
        wait_for(lambda: "rejected the password" in obs.client.last_error, AFTER_CONFIGURE, "auth error")
        with self.assertRaisesRegex(RuntimeError, "OBS rejected the password"):
            obs.record({"mode": "start"})
        self.value("connected", "off")

    def test_missing_password(self):
        obs.configure({"url": self.server.url, "password": ""})
        wait_for(lambda: "needs a password" in obs.client.last_error, AFTER_CONFIGURE, "password needed")


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
             [("GetSceneTransitionList", None), ("SetCurrentSceneTransition", {"transitionName": "Fade"}),
              ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300})]),
            (obs.set_transition, {"transition": "Swipe"}, [("SetCurrentSceneTransition", {"transitionName": "Swipe"})]),
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
             [("GetMonitorList", None),
              ("OpenVideoMixProjector", {"monitorIndex": 1, "videoMixType": "OBS_WEBSOCKET_VIDEO_MIX_TYPE_PROGRAM"})]),
            (obs.projector, {"what": "multiview"}, [("OpenVideoMixProjector", {"videoMixType": "OBS_WEBSOCKET_VIDEO_MIX_TYPE_MULTIVIEW"})]),
            (obs.projector, {"what": "source", "source": "Webcam"}, [("OpenSourceProjector", {"sourceName": "Webcam"})]),
            (obs.profile, {"profile": "Streaming"}, [("SetCurrentProfile", {"profileName": "Streaming"})]),
            (obs.scene_collection, {"collection": "Podcast"}, [("SetCurrentSceneCollection", {"sceneCollectionName": "Podcast"})]),
            (obs.caption, {"text": "Hello"}, [("SendStreamCaption", {"captionText": "Hello"})]),
            (obs.send_request, {"request": "GetVersion"}, [("GetVersion", None)]),
            (obs.send_request, {"request": "SetCurrentProgramScene", "data": '{"sceneName": "A"}'},
             [("SetCurrentProgramScene", {"sceneName": "A"})]),
            (obs.send_request_value, {"request": "SetInputAudioSyncOffset", "data": '{"inputName": "Mic", "inputAudioSyncOffset": {value}}', "value": 37},
             [("SetInputAudioSyncOffset", {"inputName": "Mic", "inputAudioSyncOffset": 37})]),
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

    def use_transition(self, name, duration=300):
        """Studio Mode on, with `name` as OBS's current transition."""
        self.server.state.update(studio=True, transition=name, duration=duration)
        self.server.event("CurrentSceneTransitionChanged", {"transitionName": name})
        self.value("transition", name)

    def restored(self, count):
        wait_for(lambda: len(self.server.requests) == count, message="the transition put back")
        time.sleep(0.05)  # and nothing more
        self.assertEqual(len(self.server.requests), count)

    def test_transition_with_override_puts_the_old_one_back(self):
        self.use_transition("Fade", 300)
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Swipe", "duration": 500}))
        self.assertEqual(self.server.requests[:5], [
            ("GetCurrentSceneTransition", None),
            ("SetCurrentSceneTransition", {"transitionName": "Swipe"}),
            ("GetCurrentSceneTransition", None),  # once OBS shows Swipe: the duration it shares
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 500}),
            ("TriggerStudioModeTransition", None),
        ])
        self.restored(7)
        self.assertEqual(self.server.requests[5:], [
            ("SetCurrentSceneTransition", {"transitionName": "Fade"}),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300}),
        ])

    def test_one_off_transition_over_a_fixed_one_puts_the_shared_duration_back(self):
        # Real OBS reports no duration while Cut is current, but has one, shared by all.
        self.use_transition("Cut", 300)
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Fade", "duration": 800}))
        self.restored(7)
        self.assertEqual(self.server.requests[3:], [
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 800}),
            ("TriggerStudioModeTransition", None),
            ("SetCurrentSceneTransition", {"transitionName": "Cut"}),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300}),
        ])
        wait_for(lambda: self.server.state["duration"] == 300 and self.server.state["transition"] == "Cut")

    def test_one_off_transition_that_is_already_current_isnt_switched_to(self):
        self.use_transition("Fade", 300)
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Fade"}))
        self.restored(2)
        self.assertEqual(self.server.requests, [("GetCurrentSceneTransition", None), ("TriggerStudioModeTransition", None)])
        # With a duration, only the duration is set and put back.
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Fade", "duration": 800}))
        self.restored(4)
        self.assertEqual(self.server.requests, [
            ("GetCurrentSceneTransition", None),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 800}),
            ("TriggerStudioModeTransition", None),
            ("SetCurrentSceneTransitionDuration", {"transitionDuration": 300}),
        ])

    def test_one_off_duration_is_left_out_when_obs_doesnt_switch_in_time(self):
        # Without the one-off transition current, OBS can't say what the shared duration was.
        self.use_transition("Cut", 300)
        self.server.handlers["SetCurrentSceneTransition"] = lambda d: None
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Fade", "duration": 800}))
        self.restored(5)
        self.assertNotIn("SetCurrentSceneTransitionDuration", [t for t, _ in self.server.requests])
        self.assertEqual(self.server.state["duration"], 300)

    def test_a_duration_is_left_out_for_a_transition_with_a_fixed_length(self):
        self.use_transition("Fade", 300)
        self.assertIsNone(self.run_action(obs.transition_now, {"transition": "Cut", "duration": 500}))
        self.restored(5)
        self.assertEqual(self.server.requests, [
            ("GetCurrentSceneTransition", None),
            ("SetCurrentSceneTransition", {"transitionName": "Cut"}),
            ("GetCurrentSceneTransition", None),  # Cut has a fixed length: no duration to set
            ("TriggerStudioModeTransition", None),
            ("SetCurrentSceneTransition", {"transitionName": "Fade"}),
        ])
        # A duration alone, while Cut is current, changes nothing (and so puts nothing back).
        wait_for(lambda: self.server.state["transition"] == "Fade")
        self.use_transition("Cut", 300)
        self.assertIsNone(self.run_action(obs.transition_now, {"duration": 500}))
        self.restored(2)
        self.assertEqual(self.server.requests, [("GetCurrentSceneTransition", None), ("TriggerStudioModeTransition", None)])
        # Choosing Cut with a duration leaves the shared duration alone too.
        self.assertIsNone(self.run_action(obs.set_transition, {"transition": "Cut", "duration": 500}))
        self.assertEqual(self.server.requests, [("GetSceneTransitionList", None),
                                                ("SetCurrentSceneTransition", {"transitionName": "Cut"})])
        self.assertEqual(self.server.state["duration"], 300)
        self.assertIsNone(self.run_action(obs.set_transition, {"transition": "Fade", "duration": 1200}))
        self.assertEqual(self.server.requests[-1], ("SetCurrentSceneTransitionDuration", {"transitionDuration": 1200}))

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

        # What OBS actually answers when the replay buffer is turned off in its settings.
        def not_available(d):
            raise Fail(604, "Replay buffer is not available.")
        self.server.handlers["ToggleReplayBuffer"] = not_available
        with self.assertRaisesRegex(RuntimeError, r"replay buffer is turned off in OBS \(Settings → Output"):
            self.run_action(obs.replay_buffer, {"mode": "toggle"})

        # Other 604s are OBS's own words.
        def busy(d):
            raise Fail(604, "Not now.")
        self.server.handlers["StartVirtualCam"] = busy
        with self.assertRaisesRegex(RuntimeError, "OBS: Not now."):
            self.run_action(obs.virtualcam, {"mode": "start"})

    def test_stream_and_record_actions(self):
        state = self.server.state
        cases = [
            (obs.stream, {"mode": "start"}, "StartStream", lambda: state["streaming"]),
            (obs.stream, {"mode": "stop"}, "StopStream", lambda: not state["streaming"]),
            (obs.stream, {"mode": "toggle"}, "ToggleStream", lambda: state["streaming"]),
            (obs.stream, {}, "ToggleStream", lambda: not state["streaming"]),
            (obs.record, {"mode": "start"}, "StartRecord", lambda: self.values["recording"] == "on"),
            (obs.record, {"mode": "pause"}, "PauseRecord", lambda: state["paused"]),
            (obs.record, {"mode": "resume"}, "ResumeRecord", lambda: not state["paused"]),
            (obs.record, {"mode": "pause-toggle"}, "ToggleRecordPause", lambda: state["paused"]),
            (obs.record, {"mode": "pause-toggle"}, "ToggleRecordPause", lambda: not state["paused"]),
            (obs.record, {"mode": "stop"}, "StopRecord", lambda: not state["recording"]),
            (obs.record, {}, "ToggleRecord", lambda: self.values["recording"] == "on"),
            (obs.record, {}, "ToggleRecord", lambda: not state["recording"]),
        ]
        for fn, settings, request_type, done in cases:
            with self.subTest(action=fn.__name__, settings=settings):
                self.assertIsNone(self.run_action(fn, settings))
                self.assertEqual(self.server.requests, [(request_type, None)])
                wait_for(done, message=f"{request_type} to finish")

    def test_pausing_while_nothing_records_says_so(self):
        # OBS answers PauseRecord and ToggleRecordPause with success while it isn't recording.
        for mode in ("pause", "pause-toggle", "resume"):
            with self.subTest(mode):
                self.assertEqual(self.run_action(obs.record, {"mode": mode}), "OBS isn't recording")
                self.assertEqual(self.server.requests, [])

    def test_pausing_a_recording_obs_cant_pause_says_why(self):
        # A recording that uses the stream's encoder: OBS says yes to a pause, and nothing happens.
        self.server.state["can_pause"] = False
        self.run_action(obs.record, {"mode": "start"})
        self.value("recording", "on")
        for mode, request_type in (("pause", "PauseRecord"), ("pause-toggle", "ToggleRecordPause")):
            with self.subTest(mode):
                began = time.monotonic()
                with self.assertRaisesRegex(RuntimeError, "couldn't pause the recording: pausing needs a recording "
                                                          r"encoder of its own, not the stream's \(Settings → Output\)"):
                    self.run_action(obs.record, {"mode": mode})
                self.assertGreaterEqual(time.monotonic() - began, obs.OUTPUT_EVENT_WAIT)
                self.assertEqual(self.server.requests, [(request_type, None), ("GetRecordStatus", None)])
        self.value("record_paused", "off")
        # It can pause: the PAUSED event answers at once, and pausing again is OBS's warning.
        self.server.state["can_pause"] = True
        began = time.monotonic()
        self.assertIsNone(self.run_action(obs.record, {"mode": "pause"}))
        self.assertLess(time.monotonic() - began, obs.OUTPUT_EVENT_WAIT)
        self.assertEqual(self.values["record_paused"], "on")
        self.assertEqual(self.run_action(obs.record, {"mode": "pause"}), "the recording is already paused")

    def test_a_stream_that_cant_connect_says_so(self):
        # OBS answers StartStream at once, goes STARTING, then STOPPED when it can't connect.
        self.server.state["stream_fails"] = True
        for settings in ({"mode": "start"}, {"mode": "toggle"}):
            with self.subTest(settings):
                began = time.monotonic()
                with self.assertRaisesRegex(RuntimeError, r"^OBS couldn't start the stream: see the message in OBS, "
                                                          r"or check Settings → Stream$"):
                    self.run_action(obs.stream, settings)
                self.assertLess(time.monotonic() - began, obs.STREAM_START_WAIT)
        self.assertNotIn("on", self.published("streaming"))

        # Still connecting when the wait is over: say so rather than that it worked.
        def starting(d):
            self.server.later(self.server.output_event("StreamStateChanged", "STARTING", False))
        self.server.handlers["StartStream"] = starting
        began = time.monotonic()
        self.assertEqual(self.run_action(obs.stream, {"mode": "start"}), "OBS is still connecting to the stream server…")
        self.assertLess(time.monotonic() - began, obs.STREAM_START_WAIT + 0.5)

    def test_split_and_chapter_while_paused(self):
        def refuse(comment):
            def handler(d):
                raise Fail(702, comment)
            return handler
        # What OBS answers while a recording is paused, although splitting and chapters work.
        self.server.handlers["SplitRecordFile"] = refuse("Verify that file splitting is enabled in the output settings.")
        self.server.handlers["CreateRecordChapter"] = refuse("Verify that the output being used supports chapter markers.")
        with self.assertRaisesRegex(RuntimeError, "^OBS: Verify that file splitting is enabled"):
            self.run_action(obs.record_split, {})
        self.server.event("RecordStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.server.event("RecordStateChanged", {"outputActive": False, "outputState": "OBS_WEBSOCKET_OUTPUT_PAUSED"})
        self.value("record_paused", "on")
        self.assertEqual(self.run_action(obs.record_split, {}), "the recording is paused; resume it first")
        self.assertEqual(self.run_action(obs.record_chapter, {"name": "x"}), "the recording is paused; resume it first")

    def test_projector_on_a_screen_obs_doesnt_have(self):
        # OBS answers success for a screen it hasn't got and opens nothing.
        with self.assertRaisesRegex(RuntimeError, "^OBS has no screen 3; it has 2$"):
            self.run_action(obs.projector, {"what": "program", "monitor": "3"})
        self.assertEqual(self.server.requests, [("GetMonitorList", None)])
        self.assertIsNone(self.run_action(obs.projector, {"what": "source", "source": "Cam", "monitor": "1"}))
        self.assertEqual(self.server.requests[-1], ("OpenSourceProjector", {"monitorIndex": 0, "sourceName": "Cam"}))

    def test_refusals_without_a_comment_are_explained_like_the_built_in_obs(self):
        def missing(d):
            raise Fail(600)
        self.server.handlers["SetCurrentProgramScene"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS: there's no scene called “Gone”$"):
            self.run_action(obs.switch_scene, {"scene": "Gone"})
        self.server.handlers["ToggleInputMute"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS: there's no source called “Mic”$"):
            self.run_action(obs.mute, {"input": "Mic"})
        # What real OBS answers for a profile or scene collection it hasn't got: 600, no comment.
        self.server.handlers["SetCurrentProfile"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS: there's no profile called “Nope”$"):
            self.run_action(obs.profile, {"profile": "Nope"})
        self.server.handlers["SetCurrentSceneCollection"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS: there's no scene collection called “Nope”$"):
            self.run_action(obs.scene_collection, {"collection": "Nope"})

        def busy(d):
            raise Fail(604)
        self.server.handlers["StartVirtualCam"] = busy
        with self.assertRaisesRegex(RuntimeError, "^OBS: it can't do that right now$"):
            self.run_action(obs.virtualcam, {"mode": "start"})

        # A request naming a scene and a source doesn't say which was missing.
        self.assertEqual(
            str(obs.ObsError("GetSceneItemId", 600, None, {"sceneName": "Main", "sourceName": "Cam"})),
            "OBS: it couldn't find that",
        )
        self.assertEqual(str(obs.ObsError("GetInputVolume", 600)), "OBS: it couldn't find that")
        # OBS's own words come first, and codes without a plain message keep the number.
        self.assertEqual(str(obs.ObsError("StartVirtualCam", 604, "Not now.")), "OBS: Not now.")
        self.assertEqual(str(obs.ObsError("SplitRecordFile", 702)), "OBS refused SplitRecordFile (error 702)")

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

    def test_slow_multi_request_actions_answer_in_time(self):
        def slow(answer):
            def handler(d):
                time.sleep(0.4)
                return answer
            return handler

        self.server.handlers["GetSceneItemId"] = slow({"sceneItemId": 7})
        self.server.handlers["GetSceneItemEnabled"] = slow({"sceneItemEnabled": True})
        self.server.handlers["SetCurrentSceneCollection"] = slow({})
        self.server.handlers["PauseRecord"] = slow({})  # and then no PAUSED: it waits for one too
        self.server.handlers["GetSceneTransitionList"] = slow({"transitions": []})
        self.server.event("RecordStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.value("recording", "on")
        old = obs.ACTION_DEADLINE
        obs.ACTION_DEADLINE = 0.3
        try:
            began = time.monotonic()
            self.assertEqual(self.run_action(obs.source_visibility, {"source": "Cam"}), obs.STILL_WORKING)
            self.assertLess(time.monotonic() - began, 0.6)
            # It finishes in the background.
            wait_for(lambda: ("SetSceneItemEnabled", {"sceneName": "Live", "sceneItemId": 7, "sceneItemEnabled": False})
                     in self.server.requests, message="the toggle to finish")
            self.assertIn("still switching", self.run_action(obs.scene_collection, {"collection": "Podcast"}))
            wait_for(lambda: ("SetCurrentSceneCollection", {"sceneCollectionName": "Podcast"}) in self.server.requests)
            for fn, settings in ((obs.record, {"mode": "pause"}), (obs.set_transition, {"transition": "Fade", "duration": 500})):
                with self.subTest(action=fn.__name__):
                    began = time.monotonic()
                    self.assertEqual(self.run_action(fn, settings), obs.STILL_WORKING)
                    self.assertLess(time.monotonic() - began, 0.6)
        finally:
            obs.ACTION_DEADLINE = old

    def test_source_visibility_missing_source(self):
        def missing(d):
            raise Fail(600, "No scene items were found in the specified scene by that name or offset.")
        self.server.handlers["GetSceneItemId"] = missing
        with self.assertRaisesRegex(RuntimeError, "no source “Cam” in the scene “Live”"):
            self.run_action(obs.source_visibility, {"source": "Cam"})

    def test_source_visibility_missing_scene(self):
        def missing(d):
            # Real OBS: the same code for a missing scene as for a missing source.
            if d["sceneName"] not in self.server.state["scenes"] + self.server.state["groups"]:
                raise Fail(600, "No source was found by the name of `%s`." % d["sceneName"])
            raise Fail(600, "No scene items were found in the specified scene by that name or offset.")
        self.server.handlers["GetSceneItemId"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS has no scene “Nowhere”$"):
            self.run_action(obs.source_visibility, {"scene": "Nowhere", "source": "Cam"})
        # Scenes made (or renamed) since connecting count as scenes, and so do groups; a scene
        # removed since doesn't.
        self.server.state["scenes"].append("Talk")
        self.server.event("SceneCreated", {"sceneName": "Talk", "sceneUuid": "u", "isGroup": False})
        self.server.state["groups"].append("Cams")
        self.server.event("SceneCreated", {"sceneName": "Cams", "sceneUuid": "g", "isGroup": True})
        self.server.state["scenes"][0] = "Live R"
        self.server.event("SceneNameChanged", {"sceneUuid": "u", "oldSceneName": "Live", "sceneName": "Live R"})
        self.value("scene", "Live R")
        for scene in ("Talk", "Cams", "Live R"):
            with self.assertRaisesRegex(RuntimeError, f"^OBS has no source “Cam” in the scene “{scene}”$"):
                self.run_action(obs.source_visibility, {"scene": scene, "source": "Cam"})
        with self.assertRaisesRegex(RuntimeError, "^OBS has no scene “Live”$"):
            self.run_action(obs.source_visibility, {"scene": "Live", "source": "Cam"})
        self.server.state["scenes"].remove("Talk")
        self.server.event("SceneRemoved", {"sceneName": "Talk", "sceneUuid": "u", "isGroup": False})
        with self.assertRaisesRegex(RuntimeError, "^OBS has no scene “Talk”$"):
            self.run_action(obs.source_visibility, {"scene": "Talk", "source": "Cam"})

    def test_without_the_group_list_a_missing_item_names_the_source(self):
        # Groups count as scenes. Without OBS's list of them, a name the scene list lacks may
        # be a group, so the plugin doesn't say the scene is missing.
        def not_ready(d):
            raise Fail(207, "OBS is not ready to perform the request.")
        self.server.handlers["GetGroupList"] = not_ready
        self.server.state["groups"].append("Cams")
        self.resync()

        def missing(d):
            raise Fail(600, "No scene items were found in the specified scene by that name or offset.")
        self.server.handlers["GetSceneItemId"] = missing
        with self.assertRaisesRegex(RuntimeError, "^OBS has no source “Cam” in the scene “Cams”$"):
            self.run_action(obs.source_visibility, {"scene": "Cams", "source": "Cam"})

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
        self.server.event("RecordStateChanged", {"outputActive": False, "outputState": "OBS_WEBSOCKET_OUTPUT_PAUSED", "outputPath": None})
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

    def record_event(self, state, active):
        self.server.event("RecordStateChanged", {"outputActive": active, "outputState": "OBS_WEBSOCKET_OUTPUT_" + state,
                                                 "outputPath": None})

    def test_a_paused_recording_is_still_recording(self):
        # OBS sends outputActive false with PAUSED. The recording is on, and its time holds.
        self.server.state["recording"] = True
        self.record_event("STARTING", False)
        self.record_event("STARTED", True)
        obs.poll()
        self.value("record_time", "01:00:00")
        self.value("recording", "on")
        self.sent.clear()
        self.record_event("PAUSED", False)
        self.value("record_paused", "on")
        self.server.state["paused"] = True
        obs.poll()  # agrees
        self.record_event("RESUMED", True)
        self.value("record_paused", "off")
        self.assertEqual(self.published("recording"), [], "still on")
        self.assertEqual(self.published("record_time"), [], "held while paused, not back to 00:00:00")

    def test_a_stopping_output_doesnt_flicker_on_again(self):
        # While a recording stops, OBS's GetRecordStatus says active for about 1.2 s more. A
        # poll then mustn't undo the STOPPING event (seen on real OBS: off, on, off).
        self.server.state["stop_takes"] = 0.5
        for key, fn in (("recording", obs.record), ("streaming", obs.stream)):
            with self.subTest(key):
                self.sent.clear()
                fn({"mode": "start"})
                self.value(key, "on")
                fn({"mode": "stop"})
                self.value(key, "off")
                self.assertTrue(self.server.state[key], "OBS is still stopping it")
                obs.poll()
                self.assertEqual(self.values[key], "off")
                self.assertEqual(self.values["record_time"], "00:00:00")
                wait_for(lambda: not self.server.state[key], message="stopped")
                obs.poll()
                self.assertEqual(self.published(key), ["on", "off"])

    def test_an_event_during_a_poll_beats_the_poll(self):
        # The recording pauses, or finishes stopping, while OBS answers the poll with what it
        # had: the event is newer, although neither leaves a start or stop under way.
        self.server.state["recording"] = True
        self.record_event("STARTED", True)
        self.value("recording", "on")
        stale = {"outputActive": True, "outputPaused": False, "outputTimecode": "00:00:05.000"}

        def paused_meanwhile(d):
            self.record_event("PAUSED", False)
            return stale
        self.server.handlers["GetRecordStatus"] = paused_meanwhile
        obs.poll()
        self.assertEqual(self.values["record_paused"], "on")
        self.assertEqual(self.values["recording"], "on")

        self.record_event("RESUMED", True)
        self.record_event("STOPPING", False)
        self.value("recording", "off")

        def stopped_meanwhile(d):
            self.record_event("STOPPED", False)
            return stale
        self.server.handlers["GetRecordStatus"] = stopped_meanwhile
        obs.poll()
        self.assertEqual(self.values["recording"], "off")
        self.assertEqual(self.values["record_time"], "00:00:00")

    def test_polling_trusts_obs_again_when_a_stop_never_finishes(self):
        # A STOPPING with no STOPPED after it holds polling off for UNDER_WAY_FOR, not for good.
        clock = [100.0]
        obs.live.clock = lambda: clock[0]
        self.addCleanup(setattr, obs.live, "clock", time.monotonic)
        self.server.state["recording"] = True
        self.record_event("STARTED", True)
        self.value("recording", "on")
        self.record_event("STOPPING", False)
        self.value("recording", "off")  # noted at 100 s
        clock[0] += obs.UNDER_WAY_FOR - 1
        obs.poll()
        self.assertEqual(self.values["recording"], "off")
        clock[0] += 2
        obs.poll()
        self.value("recording", "on")
        self.value("record_time", "01:00:00")

    def test_a_resync_while_a_recording_stops_keeps_it_off(self):
        # A scene collection change reads everything again, as a poll does, while a recording
        # that is stopping still reads as active.
        self.server.state["recording"] = True
        self.record_event("STARTED", True)
        self.value("recording", "on")
        self.record_event("STOPPING", False)
        self.value("recording", "off")
        self.sent.clear()
        self.resync()
        self.assertEqual(self.published("recording"), [])
        self.assertEqual(self.published("record_time"), [])

    def test_a_reconnecting_stream_is_still_streaming(self):
        # OBS sends outputActive false while the stream reconnects, and once it has.
        self.server.state["streaming"] = True
        self.server.event("StreamStateChanged", {"outputActive": True, "outputState": "OBS_WEBSOCKET_OUTPUT_STARTED"})
        self.value("streaming", "on")
        obs.poll()
        self.value("stream_time", "00:01:02")
        self.sent.clear()
        for state in ("RECONNECTING", "RECONNECTED"):
            self.server.event("StreamStateChanged", {"outputActive": False, "outputState": "OBS_WEBSOCKET_OUTPUT_" + state})
        self.server.event("CurrentProfileChanged", {"profileName": "Podcast"})  # after them
        self.value("profile", "Podcast")
        self.assertEqual(self.published("streaming"), [], "still on")
        self.assertEqual(self.published("stream_time"), [], "not back to 00:00:00")

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

    def test_renaming_the_live_scene_renames_the_values(self):
        self.server.handlers["GetCurrentPreviewScene"] = lambda d: {"sceneName": "Live"}
        self.server.event("StudioModeStateChanged", {"studioModeEnabled": True})
        self.value("preview_scene", "Live")
        self.watch("scene_active:Live", "scene_active:Live R", "scene_preview:Live R")
        self.value("scene_active:Live", "on")
        self.value("scene_active:Live R", "off")
        self.server.state.update(program="Live R", preview="Live R", scenes=["Live R"])
        self.server.event("SceneNameChanged", {"sceneUuid": "u", "oldSceneName": "Live", "sceneName": "Live R"})
        self.value("scene", "Live R")
        self.value("preview_scene", "Live R")
        self.value("scene_active:Live R", "on")
        self.value("scene_preview:Live R", "on")
        self.value("scene_active:Live", "off")
        # Renaming another scene leaves them alone.
        self.server.event("SceneNameChanged", {"sceneUuid": "v", "oldSceneName": "Other", "sceneName": "Other R"})
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0)
        self.assertEqual(self.values["scene"], "Live R")

    def test_a_renamed_item_no_longer_moves_its_old_names_value(self):
        ids = {("Live", "Cam"): 7}

        def item_id(d):
            if (d["sceneName"], d["sourceName"]) not in ids:
                raise Fail(600, "No scene items were found in the specified scene by that name or offset.")
            return {"sceneItemId": ids[(d["sceneName"], d["sourceName"])]}
        self.server.handlers["GetSceneItemId"] = item_id
        self.server.handlers["GetSceneItemEnabled"] = lambda d: {"sceneItemEnabled": True}
        self.watch("item_visible:Live/Cam", "item_visible:Live/Cam R")
        self.value("item_visible:Live/Cam", "on")
        self.value("item_visible:Live/Cam R", obs.UNKNOWN)
        ids = {("Live", "Cam R"): 7}  # the same item, renamed in OBS
        self.server.event("InputNameChanged", {"inputUuid": "u", "oldInputName": "Cam", "inputName": "Cam R"})
        self.value("item_visible:Live/Cam", obs.UNKNOWN)
        self.value("item_visible:Live/Cam R", "on")
        self.server.event("SceneItemEnableStateChanged", {"sceneName": "Live", "sceneUuid": "u", "sceneItemId": 7,
                                                          "sceneItemEnabled": False})
        self.value("item_visible:Live/Cam R", "off")
        self.assertEqual(self.values["item_visible:Live/Cam"], obs.UNKNOWN)
        # Looking the items up again (as after any change in a scene) keeps the value meanwhile.
        self.sent.clear()
        self.server.event("SceneItemCreated", {"sceneName": "Live", "sourceName": "Other", "sceneItemId": 8})
        wait_for(lambda: [t for t, d in self.server.batched].count("GetSceneItemEnabled") == 3, message="the lookup")
        wait_for(lambda: obs.live.jobs.unfinished_tasks == 0)
        self.assertNotIn(obs.UNKNOWN, self.published("item_visible:Live/Cam R"))

    def test_names_with_slashes_and_bars_in_two_name_arguments(self):
        # Every real name can be written: the plugin asks OBS which split it has.
        ids = {("Live", "Odd: A|B"): 5, ("Talk/Show|Live", "Cam"): 6, ("Live", "Over/lay"): 9}

        def item_id(d):
            if (d["sceneName"], d["sourceName"]) not in ids:
                raise Fail(600, "No source was found by the name of `%s`." % d["sceneName"])
            return {"sceneItemId": ids[(d["sceneName"], d["sourceName"])]}
        self.server.handlers["GetSceneItemId"] = item_id
        self.server.handlers["GetSceneItemEnabled"] = lambda d: {"sceneItemEnabled": d["sceneItemId"] != 6}

        def get_filter(d):
            if (d["sourceName"], d["filterName"]) != ("Cam", "Blur|Sharp"):
                raise Fail(600, "No filter was found in the source `Cam` with the name `x`.")
            return {"filterEnabled": True}
        self.server.handlers["GetSourceFilter"] = get_filter
        self.watch("item_visible:Live/Odd: A|B", "item_visible:Live|Odd: A|B", "item_visible:Talk/Show|Live/Cam",
                   "item_visible:Live|Over/lay", "item_visible:Live/Over/lay", "filter_enabled:Cam/Blur|Sharp")
        self.value("item_visible:Live/Odd: A|B", "on")  # the README's form: the first '/' splits
        self.value("item_visible:Live|Odd: A|B", "on")  # the old '|' form still works
        self.value("item_visible:Talk/Show|Live/Cam", "off")  # a scene with both in its name
        self.value("item_visible:Live|Over/lay", "on")
        self.value("item_visible:Live/Over/lay", "on")
        self.value("filter_enabled:Cam/Blur|Sharp", "on")
        self.server.event("SceneItemEnableStateChanged", {"sceneName": "Talk/Show|Live", "sceneUuid": "u",
                                                          "sceneItemId": 6, "sceneItemEnabled": True})
        self.value("item_visible:Talk/Show|Live/Cam", "on")
        self.server.event("SourceFilterEnableStateChanged", {"sourceName": "Cam", "filterName": "Blur|Sharp",
                                                             "filterEnabled": False})
        self.value("filter_enabled:Cam/Blur|Sharp", "off")

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
        self.assertEqual([t for t, _ in self.server.batched].count("GetStats"), 1, "only in the initial sync")
        obs.poll()
        self.value("fps", "60")
        self.value("cpu", "3.1")
        self.value("memory", "513")
        self.value("dropped_frames", "0.3")  # (1 + 2) / (900 + 100), rendering and encoding
        self.value("network_dropped_frames", "0")  # not streaming
        self.value("stream_time", "00:00:00")
        self.value("bitrate", "0")
        self.server.state["streaming"] = True
        self.server.state["recording"] = True
        self.server.state["stream_bytes"] = 1_000_000
        self.server.state["stream_skipped"] = 6
        obs.poll()
        self.value("streaming", "on")
        self.value("network_dropped_frames", "1.5")  # 6 / 400, from GetStreamStatus
        self.value("dropped_frames", "0.3")
        self.value("stream_time", "00:01:02")
        self.value("record_time", "01:00:00")
        clock[0] += 2.0
        self.server.state["stream_bytes"] = 1_000_000 + 1_500_000  # 1.5 MB in 2 s = 6000 kbps
        obs.poll()
        self.value("bitrate", "6000")
        stats = [t for t, _ in self.server.batched if t == "GetStats"]
        self.assertEqual(len(stats), 4)  # the initial sync and three polls

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
