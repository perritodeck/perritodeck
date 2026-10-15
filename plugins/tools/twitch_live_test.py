#!/usr/bin/env python3
"""Run the Twitch plugin's real-world test plan (docs/testing/twitch.md, section 3) through
the plugin's own stdio protocol, and write a JSON pass/fail report.

    py -3 plugins/tools/twitch_live_test.py --broadcaster T --viewer V --raid-target R --main-channel M
    py -3 plugins/tools/twitch_live_test.py --broadcaster T --only A,B --auto-only
    py -3 plugins/tools/twitch_live_test.py --dry-run-fake          # against the fakes
    py -3 plugins/tools/twitch_live_test.py --list                  # the cases, in order

It starts plugins/twitch/plugin.py the way the host does (hello with the settings, watch,
run; it reads result, value and log), with PERRITODECK_PLUGIN_DATA in a test folder of its
own, never PerritoDeck's real data folder. By default the plugin runs under
twitch_trace.py, which records every HTTP call with tokens redacted; --no-trace runs
plugin.py itself. The cases run in the plan's order (its "Order" list).

Safety rules:
- Signing in is the device code flow. The harness shows the code and waits for a person to
  approve it in a browser; it never enters a password or a code anywhere. A sign-in
  approved by another account, or not approved in time, is signed out at once (the
  plugin's Sign out cancels the attempt and revokes the token).
- Before every case with a public effect (chat, announcements, title, category, tags and
  labels, clips, markers, raids, shoutouts, moderation, ads, polls, predictions, rewards)
  it stops, lists each action and the channel it acts in, and runs the case only after
  you type "yes". --auto-only skips those cases.
- Before anything risky it checks that the signed-in account is the test broadcaster
  (--broadcaster), from tokens.json and by asking Twitch (GET /users).
- Before the first risky case it looks the test viewer and the raid target up on Twitch
  (both must exist, and the viewer must follow the test broadcaster), shows their IDs and
  creation dates, and goes on only after a typed "yes" that these are the test accounts.
- Moderation acts only in the test broadcaster's channel and only on the test viewer
  (--viewer); G4 also bans the test broadcaster itself (Twitch refuses) and a made-up
  name. Raids go only to the raid target (--raid-target); shoutouts to the raid target
  (or the test broadcaster itself, H5); chat only to the test broadcaster's and the raid
  target's channels. The harness refuses anything else.
- When the broadcaster is the main channel (--main-channel), moderation, chat modes,
  Shield Mode, AutoMod, clearing chat, warnings and raids are always refused, including
  the ones a person would press by hand (F2, G9, H3). With
  --no-skip-affiliate (an Affiliate's channel, often the main one) every risky case
  outside I, J and K is refused unless --only names it.
- A case that stops early (Ctrl+C, a refusal, a step it can't do) undoes what it did:
  unban the viewer, remove mod and VIP, chat modes and Shield Mode off, AutoMod back,
  cancel the raid. Each undo goes through the same checks.
- A made-up name (B1, G4) is random letters and digits; GET /users must say no account
  has it before it's used.
- Tokens are never printed or written by the harness; the trace redacts them too, and the
  report leaves out the device code.

The harness itself calls Twitch only for GET /users and GET /channels/followers (the
checks above) and GET /chat/settings (F1), and M5 repeats GET /users to use up the rate
limit. --skip-affiliate (the default) reports sections I, J and K as "needs an Affiliate
channel". Standard library only.
"""

import argparse
import itertools
import json
import os
import random
import re
import shutil
import stat
import string
import subprocess
import sys
import tempfile
import threading
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import twitch_trace  # noqa: E402
from twitch_common import (CLIENT_ID_ENV, DEFAULT_TEST_FOLDER, HELIX_URL, LOGIN, TWITCH_DIR, HelixError,  # noqa: E402
                           Redactor, folder_problem, helix, location_problem, login_key, manifest, message_of,
                           plugin_data, plugin_module, prepare_test_folder, read_tokens, token_key,
                           write_json_atomically)

PLUGIN_PY = os.path.join(TWITCH_DIR, "plugin.py")
TRACE_PY = os.path.join(HERE, "twitch_trace.py")
ACTION_TIMEOUT = 10.0  # the host gives an action this long
NOTHING = "–"  # the plugin's placeholder

# What a case or an action does to the world.
READ, PRIVATE, PUBLIC, MODERATION = "read", "private", "public", "moderation"
RANK = {READ: 0, PRIVATE: 1, PUBLIC: 2, MODERATION: 3}
# How the plan runs a case.
AUTO, SEMI, MANUAL = "auto", "semi", "manual"
# Results.
PASS, FAIL, RECORDED, SKIPPED, REFUSED, DONE = "pass", "fail", "recorded", "skipped", "refused", "done"
MANUAL_RESULT, NEEDS_AFFILIATE = "manual", "needs-affiliate"
UNDO_FAILED = "undo-failed"  # a step record: undoing an early-stopped case's change didn't work
AFFILIATE_SECTIONS = ("I", "J", "K")
CASE_ID = re.compile(r"^[A-N]\d+$")

# Every action of the plugin, and what it does. Anything not listed counts as moderation,
# and the gate refuses it.
ACTION_EFFECT = {
    "sign-in": PRIVATE, "sign-out": PRIVATE, "open-last-clip": PRIVATE, "chat-color": PRIVATE,
    "marker": PUBLIC, "clip": PUBLIC, "set-info": PUBLIC, "preset": PUBLIC, "tags": PUBLIC, "label": PUBLIC,
    "branded": PUBLIC, "ad": PUBLIC, "snooze-ad": PUBLIC, "chat": PUBLIC, "chat-saved": PUBLIC, "announce": PUBLIC,
    "raid": PUBLIC, "cancel-raid": PUBLIC, "shoutout": PUBLIC, "reward-create": PUBLIC, "reward": PUBLIC,
    "redemption": PUBLIC, "poll-start": PUBLIC, "poll-end": PUBLIC, "prediction-start": PUBLIC,
    "prediction-lock": PUBLIC, "prediction-resolve": PUBLIC, "prediction-cancel": PUBLIC,
    "chat-mode": MODERATION, "shield": MODERATION, "automod": MODERATION, "clear-chat": MODERATION,
    "ban": MODERATION, "unban": MODERATION, "warn": MODERATION, "vip": MODERATION, "mod": MODERATION,
}
MOD_CHANNEL_ACTIONS = {"chat-mode", "shield", "automod", "clear-chat", "ban", "unban", "warn"}
USER_ACTIONS = {"ban", "unban", "warn", "vip", "mod"}
POST_ACTIONS = {"chat", "chat-saved", "announce", "clip"}
# Never on the main channel (the plan's "Never on the main channel" list): moderation,
# chat modes, Shield Mode, AutoMod, clearing chat, warnings and raids.
NEVER_ON_MAIN = MOD_CHANNEL_ACTIONS | USER_ACTIONS | {"raid"}
# The device code in the plugin's answer to Sign in ("... and enter ABCD1234") and in its
# log ("... and enter the code ABCD1234").
USER_CODE = re.compile(r"\benter (?:the code )?([A-Za-z0-9-]{4,})\s*$")
OFF_STATES = ("off", "remove", "disable")

ON_OFF = r"on|off"
DIGITS = r"\d+"
ANY = r"(?s).+"
UPTIME = r"offline|\d+m|\d+h \d\dm"
PH = re.escape(NOTHING)


class Exactly(str):
    """A value to match literally (placeholders filled in), not as a pattern."""


class SkipCase(Exception):
    def __init__(self, reason, status=SKIPPED):
        super().__init__(reason)
        self.status = status


class Refusal(Exception):
    """A safety rule stopped the harness from sending something."""


def made_up_login(rng=random.SystemRandom()):
    """A name that looks like a Twitch login (16 letters and digits, starting with a
    letter); GET /users still has to confirm that nobody has it."""
    return rng.choice(string.ascii_lowercase) + "".join(
        rng.choice(string.ascii_lowercase + string.digits) for _ in range(15))


def random_client_id(rng=random.SystemRandom()):
    return "".join(rng.choice(string.ascii_lowercase + string.digits) for _ in range(30))


# --- the plugin process -------------------------------------------------------------------------


class Answer:
    def __init__(self, ok, message, seconds, mark, timed_out=False):
        self.ok, self.message, self.seconds, self.mark, self.timed_out = ok, message, seconds, mark, timed_out

    def as_dict(self):
        d = {"ok": self.ok, "message": self.message, "seconds": round(self.seconds, 2)}
        if self.timed_out:
            d["timed_out"] = True
        return d


class PluginProcess:
    """plugin.py (or twitch_trace.py run) as a child process, spoken to over stdio."""

    def __init__(self, command, data_dir, stderr_path, on_log=None):
        self.command, self.data_dir, self.stderr_path, self.on_log = command, data_dir, stderr_path, on_log
        self.cond = threading.Condition()
        self.send_lock = threading.Lock()
        self.seq = 0
        self.values = {}
        self.value_events = []  # (seq, value id, value)
        self.logs = []  # (seq, level, message)
        self.results = {}
        self.ids = itertools.count(1)
        self.proc = None
        self.err = None
        self.ready = False
        self.exited = False
        self.watching = []
        self.started_at = 0  # the mark when this process started

    def start(self, settings):
        env = dict(os.environ, PERRITODECK_PLUGIN_DATA=self.data_dir, PYTHONUTF8="1", PYTHONIOENCODING="utf-8")
        self.err = open(self.stderr_path, "a", encoding="utf-8")
        with self.cond:
            self.ready = self.exited = False
            self.values = {}
            self.started_at = self.seq
        self.proc = subprocess.Popen(self.command, cwd=TWITCH_DIR, env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=self.err, text=True, encoding="utf-8",
                                     errors="replace", bufsize=1)
        threading.Thread(target=self._read, args=(self.proc,), daemon=True).start()
        self.send({"event": "hello", "protocol": 1, "host": "twitch-live-test", "settings": settings, "features": []})
        if not self.wait(lambda: self.ready or self.exited, 30):
            raise RuntimeError("the plugin didn't say ready within 30 s")
        if self.exited:
            raise RuntimeError(f"the plugin exited at start; see {self.stderr_path}")
        if self.watching:
            self.send({"event": "watch", "values": self.watching})

    def _read(self, proc):
        for line in proc.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if not isinstance(msg, dict):
                continue
            event = msg.get("event")
            with self.cond:
                self.seq += 1
                if event == "ready":
                    self.ready = True
                elif event == "value":
                    vid, value = str(msg.get("id")), str(msg.get("value"))
                    self.values[vid] = value
                    self.value_events.append((self.seq, vid, value))
                    del self.value_events[:-20000]
                elif event == "log":
                    self.logs.append((self.seq, str(msg.get("level") or "info"), str(msg.get("message") or "")))
                elif event == "result":
                    self.results[msg.get("id")] = (msg, time.monotonic())
                self.cond.notify_all()
            if event == "log" and self.on_log:
                self.on_log(str(msg.get("level") or "info"), str(msg.get("message") or ""))
        try:
            proc.stdout.close()
        except OSError:
            pass
        with self.cond:
            if proc is self.proc:
                self.exited = True
            self.cond.notify_all()

    def send(self, message):
        line = json.dumps(message, ensure_ascii=False) + "\n"
        with self.send_lock:
            try:
                self.proc.stdin.write(line)
                self.proc.stdin.flush()
            except (OSError, ValueError):
                pass

    def wait(self, predicate, timeout):
        with self.cond:
            return self.cond.wait_for(predicate, timeout)

    def mark(self):
        with self.cond:
            return self.seq

    def run(self, action, settings, timeout=ACTION_TIMEOUT):
        rid = next(self.ids)
        mark = self.mark()
        start = time.monotonic()
        self.send({"event": "run", "id": rid, "action": action, "settings": settings, "device": "twitch-live-test"})
        self.wait(lambda: rid in self.results or self.exited, timeout)
        if rid in self.results:
            msg, at = self.results[rid]
            return Answer(bool(msg.get("ok")), str(msg.get("message") or ""), at - start, mark)
        return Answer(False, "", time.monotonic() - start, mark, timed_out=True)

    def watch(self, values):
        values = sorted(set(values))
        with self.cond:
            for vid in [v for v in self.values if ":" in v and v not in values]:
                del self.values[vid]  # as the plugin forgets what it published for them
        self.watching = values
        self.send({"event": "watch", "values": values})

    def logs_since(self, seq):
        with self.cond:
            return [entry for entry in self.logs if entry[0] > seq]

    def stop(self, kill=False):
        proc = self.proc
        if proc is None:
            return
        if kill:
            proc.kill()
        else:
            self.send({"event": "shutdown"})
            try:
                proc.stdin.close()
            except OSError:
                pass
            try:
                proc.wait(10)
            except subprocess.TimeoutExpired:
                proc.kill()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            pass
        try:
            proc.stdin.close()
        except OSError:
            pass
        if self.err:
            self.err.close()
            self.err = None


# --- expectations ------------------------------------------------------------------------------


class OK:
    def __init__(self, warning=None):
        self.warning = warning  # a warning this pattern matches still counts as ok

    def judge(self, a, h):
        if a.timed_out:
            return FAIL, f"no answer within {ACTION_TIMEOUT:g} s (the host would show its timeout)"
        if not a.ok:
            return FAIL, f"failed: {a.message}"
        if a.message and not (self.warning and re.search(h.fmt(self.warning, True), a.message)):
            return FAIL, f"answered with a warning: {a.message}"
        return PASS, f"ok: {a.message}" if a.message else "ok"


class Warns:
    def __init__(self, pattern):
        self.pattern = pattern

    def judge(self, a, h):
        if a.ok and a.message and re.search(h.fmt(self.pattern, True), a.message):
            return PASS, f"warned: {a.message}"
        if a.timed_out:
            return FAIL, "no answer in time"
        return FAIL, f"expected a warning like {self.pattern!r}; got {'ok' if a.ok else 'failed'}: {a.message}"


class Fails:
    def __init__(self, pattern=None):
        self.pattern = pattern

    def judge(self, a, h):
        if a.timed_out:
            return FAIL, f"no answer within {ACTION_TIMEOUT:g} s"
        if a.ok:
            return FAIL, f"expected it to fail, but it worked{': ' + a.message if a.message else ''}"
        if self.pattern and not re.search(h.fmt(self.pattern, True), a.message):
            return FAIL, f"failed with {a.message!r}; expected {self.pattern!r}"
        return PASS, f"failed as expected: {a.message}"


class Record:
    """Any answer is the result: the plan says to record what Twitch does."""

    def judge(self, a, h):
        if a.timed_out:
            return RECORDED, f"no answer within {ACTION_TIMEOUT:g} s"
        return RECORDED, ("ok" if a.ok else "failed") + (f": {a.message}" if a.message else "")


# --- steps -------------------------------------------------------------------------------------


class Step:
    effect = READ
    optional = False  # a check by eye: asked only with --check-by-eye

    def describe(self, h):
        return type(self).__name__.lower()


class Run(Step):
    """Press a key: run an action with these settings."""

    def __init__(self, action, settings=None, expect=None, **more):
        self.action = action
        self.settings = dict(settings or {}, **more)
        self.expect = expect or OK()
        self.effect = ACTION_EFFECT.get(action, MODERATION)

    def resolved(self, h):
        return {k: (v(h) if callable(v) else h.fmt(v)) for k, v in self.settings.items()}

    def target(self, h):
        """Where it acts and on whom (what the gate looks at), known before the case runs."""
        return {k: h.fmt(v) for k, v in self.settings.items() if k in ("channel", "user") and not callable(v)}

    def describe(self, h, settings=None):
        items = (settings if settings is not None else {k: h.shown(v) for k, v in self.settings.items()}).items()
        shown = ", ".join(f"{k}=" + ("(set when it runs)" if callable(v) else repr(str(v)[:60])) for k, v in items)
        return f"run {self.action}" + (f" ({shown})" if shown else "")

    def run(self, h):
        settings = self.resolved(h)
        h.allow(self.action, settings)
        a = h.plugin.run(self.action, settings)
        if self.action == "sign-in":
            h.learn_user_code(a.message)
        h.note_undo(self.action, settings, a)
        status, detail = self.expect.judge(a, h)
        return status, detail, {"answer": a.as_dict(), "step": self.describe(h, settings)}


class Watch(Step):
    def __init__(self, *values, add=False):
        self.values, self.add = values, add

    def describe(self, h):
        return "watch " + (", ".join(h.shown(v) for v in self.values) or "nothing")

    def run(self, h):
        values = [h.fmt(v) for v in self.values]
        h.watch((h.plugin.watching + values) if self.add else values)
        return DONE, "watching " + (", ".join(values) or "nothing")


class Value(Step):
    def __init__(self, value_id, want=ANY, timeout=20, record=False, after=None, optional=False, fake_note=None):
        self.value_id, self.want, self.timeout, self.record = value_id, want, timeout, record
        self.after, self.optional, self.fake_note = after, optional, fake_note

    def describe(self, h):
        return f"value {h.shown(self.value_id)}"

    def run(self, h):
        vid = h.fmt(self.value_id)
        pattern = re.escape(h.fmt(self.want)) if isinstance(self.want, Exactly) else h.fmt(self.want, True)
        after = h.prev_mark if self.after == "previous" else None
        got = h.wait_value(vid, pattern, self.timeout, after)
        current = h.plugin.values.get(vid)
        if self.record:
            return RECORDED, f"{vid} = {current!r}"
        if h.fake is not None and self.fake_note:
            return RECORDED, f"{vid} = {current!r} ({self.fake_note})"
        if got:
            return PASS, f"{vid} = {current!r}"
        return FAIL, f"{vid} = {current!r} after {self.timeout} s; expected {self.want!r}"


class Log(Step):
    def __init__(self, pattern, timeout=15, count=None, settle=0, since="case", record=False):
        self.pattern, self.timeout, self.count, self.settle = pattern, timeout, count, settle
        self.since, self.record = since, record

    def describe(self, h):
        return f"log {h.shown(self.pattern)}"

    def run(self, h):
        rx = re.compile(h.fmt(self.pattern, True))
        mark = h.prev_mark if self.since == "previous" else h.case_mark
        found = h.plugin.wait(lambda: any(rx.search(m) for s, _, m in h.plugin.logs if s > mark), self.timeout)
        if self.count is not None:
            time.sleep(self.settle * h.scale)
        lines = [m for _, _, m in h.plugin.logs_since(mark) if rx.search(m)]
        if self.record:
            return RECORDED, (f"logged: {lines[0]}" if lines else "nothing logged")
        if not found:
            return FAIL, f"no log line like {self.pattern!r}"
        if self.count is not None and len(lines) != self.count:
            return FAIL, f"logged {len(lines)} times, expected {self.count}: {lines[0]}"
        return PASS, f"logged: {lines[0]}"


class Settings(Step):
    effect = PRIVATE

    def __init__(self, **changes):
        self.changes = changes

    def describe(self, h):
        return "settings " + ", ".join(f"{k}={'(made up)' if callable(v) else v!r}" for k, v in self.changes.items())

    def run(self, h):
        h.settings.update({k: (v(h) if callable(v) else v) for k, v in self.changes.items()})
        h.send_settings()
        return DONE, self.describe(h)


class Baseline(Step):
    effect = PRIVATE

    def describe(self, h):
        return "settings back to the start"

    def run(self, h):
        h.settings = dict(h.baseline)
        h.send_settings()
        return DONE, "settings back to the start"


class Wait(Step):
    def __init__(self, seconds, scaled=True):
        self.seconds, self.scaled = seconds, scaled

    def describe(self, h):
        return f"wait {self.seconds * (h.scale if self.scaled else 1):g} s"

    def run(self, h):
        time.sleep(self.seconds * (h.scale if self.scaled else 1))
        return DONE, self.describe(h)


class Ask(Step):
    """A question for the person: y passes, n fails, s skips."""

    def __init__(self, question, optional=False):
        self.question, self.optional = question, optional

    def describe(self, h):
        return "ask: " + h.shown(self.question)

    def run(self, h):
        if not h.person:
            return SKIPPED, f"needs a person: {h.fmt(self.question)}"
        answer = h.ask(f"{h.fmt(self.question)} [y]es / [n]o / [s]kip: ").strip().lower()
        note = h.ask("  note for the report (Enter for none): ").strip() if answer in ("y", "n", "yes", "no") else ""
        if answer in ("y", "yes"):
            return PASS, "yes" + (f"; {note}" if note else "")
        if answer in ("n", "no"):
            return FAIL, "no" + (f"; {note}" if note else "")
        return SKIPPED, "skipped by the person"


class AskText(Step):
    def __init__(self, question, key, default=None):
        self.question, self.key, self.default = question, key, default

    def describe(self, h):
        return "ask: " + h.shown(self.question)

    def run(self, h):
        if not h.person:
            if self.default is None:
                raise SkipCase(f"needs a person: {h.fmt(self.question)}", MANUAL_RESULT)
            h.answers[self.key] = self.default
            return DONE, f"{self.key} = {self.default!r} (the default)"
        answer = h.ask(f"{h.fmt(self.question)} ").strip()
        if not answer and self.default is None:
            raise SkipCase("no answer given")
        h.answers[self.key] = answer or self.default
        return DONE, f"{self.key} = {h.answers[self.key]!r}"


class Do(Step):
    """Something a person does (or, against the fakes, a stand-in for it)."""

    def __init__(self, text, fake=None, optional=False):
        self.text, self.fake, self.optional = text, fake, optional

    def describe(self, h):
        return "person: " + h.shown(self.text)

    def run(self, h):
        if h.fake is not None and self.fake is not None:
            self.fake(h)
            return DONE, "stood in for the person: " + h.fmt(self.text)
        if not h.person:
            raise SkipCase(f"needs a person: {h.fmt(self.text)}", MANUAL_RESULT)
        answer = h.ask(f"{h.fmt(self.text)}\n  Press Enter when done, or type skip: ").strip().lower()
        if answer == "skip":
            raise SkipCase("skipped by the person")
        return DONE, "done: " + h.fmt(self.text)


class Guided(Step):
    """A case a person runs; the harness records the outcome. `by_hand` are the plugin
    actions the person presses, so the main-channel rule covers them too."""

    def __init__(self, text, by_hand=()):
        self.text = text
        self.by_hand = tuple(by_hand)

    def describe(self, h):
        return "by hand: " + h.shown(self.text)

    def run(self, h):
        if not h.person:
            raise SkipCase("run by a person: " + h.fmt(self.text), MANUAL_RESULT)
        h.say("  By hand: " + h.fmt(self.text))
        answer = h.ask("  Result? [p]ass / [f]ail / [r]ecorded / [s]kip: ").strip().lower()
        if answer[:1] not in ("p", "f", "r"):
            raise SkipCase("skipped by the person", MANUAL_RESULT)
        note = h.ask("  What happened (for the report): ").strip()
        return {"p": PASS, "f": FAIL, "r": RECORDED}[answer[:1]], note or "(no note)"


class Custom(Step):
    """fn(h) does the step. `actions` are the Run steps it may run (not run from here):
    the yes prompt lists them and the preflight checks where they act."""

    def __init__(self, fn, label, effect=READ, optional=False, actions=()):
        self.fn, self.label, self.effect, self.optional = fn, label, effect, optional
        self.actions = list(actions)

    def describe(self, h):
        return h.shown(self.label)

    def run(self, h):
        out = self.fn(h)
        return out if out is not None else (DONE, h.shown(self.label))


class TraceCheck(Step):
    """A check of the HTTP trace; skipped without one. fn(h, records) -> (status, detail)."""

    def __init__(self, fn, label, timeout=0, since="case"):
        self.fn, self.label, self.timeout, self.since = fn, label, timeout, since

    def describe(self, h):
        return "trace: " + h.shown(self.label)

    def run(self, h):
        if h.trace is None:
            return SKIPPED, "needs the trace (run without --no-trace)"
        mark = {"previous": h.prev_trace, "all": 0}.get(self.since, h.case_trace)
        deadline = time.monotonic() + self.timeout * max(h.scale, 0.5)
        while True:
            status, detail = self.fn(h, h.trace.since(mark))
            if status != FAIL or time.monotonic() >= deadline:
                return status, detail
            time.sleep(0.25)


class RequireLive(Step):
    """The case needs a channel live (or offline): checked with the live value. A person
    is asked to go live when `ask`; the fakes go live on their own."""

    def __init__(self, on=True, who="me", ask=True):
        self.on, self.who, self.ask_person = on, who, ask

    def describe(self, h):
        return f"{h.shown(self.who)} {'live' if self.on else 'offline'}"

    def run(self, h):
        who = h.fmt(self.who)
        vid = f"live:{who}"
        want = "on" if self.on else "off"
        if vid not in h.plugin.watching:
            h.watch(h.plugin.watching + [vid])
        h.wait_value(vid, ON_OFF, 25)
        if h.plugin.values.get(vid) == want:
            return DONE, f"{vid} is {want}"
        if h.fake is not None:
            h.fake.set_live(h.T if who == "me" else who, self.on)
        elif self.ask_person and h.person:
            name = h.T if who == "me" else who
            Do(f"{'Go live' if self.on else 'Stop streaming'} on {name}; the harness waits for {vid} to be {want}").run(h)
        else:
            raise SkipCase(f"needs {who} {'live' if self.on else 'offline'} ({vid} is {h.plugin.values.get(vid)!r})")
        if h.wait_value(vid, re.escape(want), 300 * h.scale if h.fake is None else 30):
            return DONE, f"{vid} is {want}"
        raise SkipCase(f"{vid} didn't turn {want}")


class Restart(Step):
    effect = PRIVATE

    def __init__(self, kill=False):
        self.kill = kill

    def describe(self, h):
        return "kill and start the plugin again" if self.kill else "restart the plugin"

    def run(self, h):
        h.restart_plugin(kill=self.kill)
        return DONE, self.describe(h)


class SignIn(Step):
    """Sign in with the device code flow: the harness shows the code and waits for a
    person to approve it (the fakes approve on their own). It never enters anything."""

    effect = PRIVATE

    def __init__(self, reuse=False):
        self.reuse = reuse

    def describe(self, h):
        return f"sign in as {h.T}"

    def run(self, h):
        if self.reuse and not h.args.fresh_sign_in and h.signed_in_login() == h.T:
            return SKIPPED, f"already signed in as {h.T} (--fresh-sign-in signs in again)"
        if h.fake is None and not h.person:
            raise SkipCase("signing in needs a person (run without --auto-only)", MANUAL_RESULT)
        if h.fake is not None:
            h.fake.approve_next_sign_in()
        h.send_settings()
        h.await_start_check()
        mark = h.plugin.mark()
        a = h.plugin.run("sign-in", {}, timeout=20)
        h.learn_user_code(a.message)
        if not a.ok:
            return FAIL, f"Sign in failed: {a.message}", {"answer": a.as_dict()}
        minutes = h.args.sign_in_minutes
        h.say("")
        h.say("  " + "=" * 70)
        h.say(f"  SIGN IN AS {h.T.upper()}: {a.message}")
        h.say(f"  Use the browser profile signed in to Twitch as {h.T}: the device page approves for")
        h.say("  whoever is signed in there. The harness enters nothing; it waits up to "
              f"{minutes:g} minutes for the approval.")
        h.say("  " + "=" * 70)
        got = h.plugin.wait(lambda: sign_in_outcome(h, mark) is not None, minutes * 60)
        m = sign_in_outcome(h, mark)
        extra = {"answer": a.as_dict()}
        if not got or m is None:
            # The attempt would go on polling for half an hour, and a late approval in the
            # wrong browser profile would swap the sign-in in the middle of the run.
            h.sign_in_failed = f"no approval within {minutes:g} minutes; {h.sign_out_now()}"
            return FAIL, h.sign_in_failed, extra
        if m.group("error"):
            return FAIL, f"the sign-in didn't work: {m.group('error')}", extra
        login = m.group("login").lower()
        h.verified.clear()
        if login != h.T:
            h.wrong_account = login
            # That account's token, with every scope, mustn't stay in the test folder.
            h.sign_in_failed = f"signed in as {login}, not the test broadcaster {h.T}; {h.sign_out_now()}"
            return FAIL, (f"{h.sign_in_failed}. Every case with a public effect is refused until the right "
                          "account signs in"), extra
        h.wrong_account = h.sign_in_failed = None
        return PASS, f"signed in as {login}; the key answered in {a.seconds:.1f} s (the host allows 10 s)", extra


SIGN_IN_LINE = re.compile(r"^(?:signed in to Twitch as (?P<login>\S+)|Twitch sign-in: (?P<error>.+))$")
# What a signed-in plugin says once after it starts (its first check of the token): other
# words than an approved sign-in's, so it never reads as one's outcome.
START_LINE = "the saved Twitch sign-in is for "
# How an earlier attempt ends when Sign in is pressed again: not this attempt's outcome.
EARLIER_ATTEMPT = ("replaced by a new sign-in", "cancelled by signing out")


def sign_in_outcome(h, mark):
    """The log line that says how the sign-in pressed after `mark` went, or None yet."""
    for _, _, line in h.plugin.logs_since(mark):
        m = SIGN_IN_LINE.match(line)
        if m and m.group("error") not in EARLIER_ATTEMPT:
            return m
    return None


def settle_sign_in(h, mark, seconds):
    """For a case that presses Sign in itself (A4, A5): waits up to `seconds` for how the
    attempt pressed after `mark` ends, and tidies up as SignIn does. An attempt still
    waiting is cancelled, and a wrong account's token revoked, at once, so neither stays in
    the test folder or swaps the sign-in later in the run. Returns (login, error, note):
    at most one of login and error is set, and note says what was tidied up."""
    h.plugin.wait(lambda: sign_in_outcome(h, mark) is not None, seconds)
    m = sign_in_outcome(h, mark)
    if m is None:  # the next case that needs the sign-in signs in again
        return None, None, f"no answer within {seconds:g} s; {h.sign_out_now()}"
    if m.group("error"):
        return None, m.group("error"), ""
    login = m.group("login").lower()
    h.verified.clear()
    if login != h.T:
        h.wrong_account = login
        h.sign_in_failed = f"signed in as {login}, not the test broadcaster {h.T}; {h.sign_out_now()}"
        return login, None, h.sign_in_failed
    h.wrong_account = h.sign_in_failed = None
    return login, None, ""


def person_step(h, step):
    """A person's step after a case pressed Sign in itself: skipping it cancels the
    attempt still waiting, so it can't sign anyone in later in the run."""
    try:
        return h.do(step)
    except SkipCase as e:
        raise SkipCase(f"{e}; {h.sign_out_now()}", e.status) from None


# --- cases -------------------------------------------------------------------------------------


class Case:
    def __init__(self, cid, stage, title, how, effect, steps=(), needs=(), fake=False, signed_in=True,
                 confirm_note=None):
        self.id, self.stage, self.title, self.how, self.effect = cid, stage, title, how, effect
        self.steps = list(steps)
        self.needs = set(needs)
        self.fake = fake
        self.signed_in = signed_in
        self.confirm_note = confirm_note

    @property
    def section(self):
        return self.id[0]

    @property
    def risky(self):
        return RANK[self.effect] >= RANK[PUBLIC]


def check_definitions(cases):
    """Each case is declared at least as risky as the actions it runs, and a Custom step
    as risky as the actions it says it runs."""
    problems = []
    for case in cases:
        for step in case.steps:
            if RANK[step.effect] > RANK[case.effect]:
                problems.append(f"{case.id} is declared {case.effect} but has a {step.effect} step "
                                f"({type(step).__name__})")
            for run in getattr(step, "actions", ()):
                if RANK[run.effect] > RANK[step.effect]:
                    problems.append(f"{case.id}: a {step.effect} custom step declares {run.action}, "
                                    f"which is {run.effect}")
    return problems


def undo_for(action, settings, answers=None):
    """What an action leaves to undo if its case stops early, as (key, undo): undo is the
    (action, settings) that puts it back, or None when this action itself puts `key` back.
    None for an action that leaves nothing to undo."""
    same = {"channel": settings["channel"]} if settings.get("channel") else {}
    user = login_key(settings.get("user"))
    change = (settings.get("change") or "toggle").strip().lower()
    state = (settings.get("state") or "toggle").strip().lower()
    if action in ("ban", "unban"):
        return ("ban", user), (("unban", dict(same, user=user)) if action == "ban" else None)
    if action in ("vip", "mod"):
        return (action, user), (None if change in OFF_STATES else (action, dict(same, user=user, change="remove")))
    if action == "chat-mode":
        mode = (settings.get("mode") or "").strip().lower()
        return ("chat-mode", mode), (None if state in OFF_STATES else ("chat-mode", dict(same, mode=mode, state="off")))
    if action == "shield":
        return ("shield",), (None if state in OFF_STATES else ("shield", dict(same, state="off")))
    if action == "raid":
        return ("raid",), ("cancel-raid", {})
    if action == "cancel-raid":
        return ("raid",), None
    if action == "automod" and str((answers or {}).get("automod") or "").strip():
        return ("automod",), ("automod", dict(same, level=str(answers["automod"]).strip()))
    return None


# Custom steps and trace checks used by the cases below.


def http_since(h, records, method, path, status=None):
    return twitch_trace.http_records(records, method, path, status)


def no_calls(*pairs):
    def check(h, records):
        sent = [f"{m} {p}" for m, p in pairs if twitch_trace.http_records(records, m, p)]
        return (FAIL, "sent: " + ", ".join(sent)) if sent else (PASS, "nothing sent to Twitch for them")
    return check


def check_tokens_file(h):
    tokens = h.tokens()
    if not tokens:
        return FAIL, "no tokens.json"
    twitch = plugin_module()
    expected = set(twitch.BASE_SCOPES) | {s for _, scopes in twitch.GROUPS.values() for s in scopes}
    scopes = set(tokens.get("scope") or [])
    login = (tokens.get("login") or "").lower()
    problems = []
    if login != h.T:
        problems.append(f"login is {login or 'missing'}, not {h.T}")
    if scopes != expected:
        problems.append(f"{len(scopes)} scopes; missing {sorted(expected - scopes)}, extra {sorted(scopes - expected)}")
    if "moderator:read:automod_settings" in scopes:
        problems.append("it has moderator:read:automod_settings")
    if problems:
        return FAIL, "; ".join(problems)
    return PASS, f"login {login}; the {len(scopes)} scopes of every group, without moderator:read:automod_settings"


def check_tokens_private(h):
    path = h.tokens_path
    if not os.path.exists(path):
        return FAIL, "no tokens.json"
    if os.name == "posix":
        mode = stat.S_IMODE(os.stat(path).st_mode)
        return (PASS if mode & 0o077 == 0 else FAIL), f"mode {oct(mode)}"
    icacls = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "icacls.exe")
    if not os.path.exists(icacls):
        return SKIPPED, "icacls.exe not found"
    out = subprocess.run([icacls, path], capture_output=True, text=True, errors="replace").stdout
    user = os.environ.get("USERNAME", "")
    principals = []
    for line in out.replace(path, "").splitlines():
        line = line.strip()
        if ":(" in line:
            principals.append(line.split(":(", 1)[0].strip())
    # The user, SYSTEM and Administrators; "OWNER RIGHTS" is the file's owner (a folder made
    # owner-only, as Python's mkdtemp does on Windows, grants it that way).
    others = [p for p in principals if not (p.lower().endswith(("\\system", "\\administrators"))
                                            or p.lower() in ("owner rights", "creator owner")
                                            or (user and p.lower().endswith("\\" + user.lower())))]
    shown = ", ".join("<user>" if user and p.lower().endswith("\\" + user.lower()) else p for p in principals)
    return (FAIL, f"others can read it too: {shown}") if others else (PASS, f"readable by {shown}")


def check_no_token_leak(h):
    tokens = h.tokens(learn=True) or {}
    secrets = [s for s in (tokens.get("access_token"), tokens.get("refresh_token")) if s]
    if not secrets:
        return SKIPPED, "not signed in"
    texts = {"the plugin's log": "\n".join(m for _, _, m in h.plugin.logs),
             "the values": json.dumps(h.plugin.values, ensure_ascii=False)}
    for name, path in (("the plugin's stderr", h.stderr_path), ("the trace", h.trace_path)):
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                texts[name] = f.read()
        except (OSError, TypeError):
            pass
    found = [name for name, text in texts.items() if any(s in text for s in secrets)]
    return (FAIL, "a token is in " + ", ".join(found)) if found else (PASS, "no token in " + ", ".join(texts))


def live_consistent(who):
    def check(h):
        live, viewers, up = (h.plugin.values.get(f"{v}:{who}") for v in ("live", "viewers", "uptime"))
        if live == "off" and (viewers, up) != ("0", "offline"):
            return FAIL, f"off, but viewers {viewers!r} and uptime {up!r}"
        if live == "on" and up == "offline":
            return FAIL, "on, but uptime says offline"
        return PASS, f"live {live}, viewers {viewers}, uptime {up}"
    return check


def no_refresh_while_polling(value_id):
    def check(h):
        before = token_key((h.tokens() or {}).get("access_token"))
        time.sleep(65 * h.scale)  # two polls of the 60 s job
        after = token_key((h.tokens() or {}).get("access_token"))
        detail = []
        if h.trace is not None:
            records = h.trace.since(h.case_trace)
            refreshes = [r for r in http_since(h, records, "POST", "/oauth2/token")
                         if twitch_trace.grant(r) == "refresh_token"]
            polls = http_since(h, records, "GET", "/helix/subscriptions")
            detail.append(f"{len(polls)} GET /helix/subscriptions, {len(refreshes)} refresh(es)")
            if refreshes:
                return FAIL, "; ".join(detail)
        if before != after:
            return FAIL, "the token changed: the plugin refreshed it"
        return PASS, "; ".join(detail + ["no refresh"])
    return check


def b8_names(h):
    names = []
    for n in [h.T, h.V, h.R, h.P] + [login_key(x) for x in (h.args.b8_channels or "").split(",")]:
        if n and n not in names and LOGIN.match(n):
            names.append(n)
    return names


def check_b8(h):
    names = b8_names(h)
    status, answer = h.own_helix("GET", "/users", [("login", n) for n in names])
    if status != 200:
        return FAIL, f"couldn't look the channels up (Twitch said {status}: {message_of(answer)})"
    known = {(u.get("login") or "").lower() for u in answer.get("data") or []}
    chosen = [n for n in names if n in known][:10]
    if len(chosen) < 2:
        raise SkipCase("fewer than two of the channel names exist; pass --b8-channels")
    trace_mark = h.trace.last_seq() if h.trace else 0
    h.do(Watch(*[f"live:{n}" for n in chosen], "live:bad name"))
    for n in chosen:
        h.do(Value(f"live:{n}", ON_OFF))
    h.do(Value("live:bad name", PH))
    h.do(Log(r"^name bad name: 'bad name' can't be a Twitch channel name"))
    time.sleep(21 * h.scale)
    notes = [f"{len(chosen)} channel(s)" + ("" if len(chosen) >= 10 else " (the plan wants 10: --b8-channels)")]
    if h.trace is not None:
        streams = http_since(h, h.trace.since(trace_mark), "GET", "/helix/streams")
        notes.append(f"{len(streams)} GET /helix/streams in about {21 * h.scale:g} s")
    short = "".join(random.choice(string.ascii_lowercase) for _ in range(3))
    mark, trace_mark = h.plugin.mark(), (h.trace.last_seq() if h.trace else 0)
    h.watch(h.plugin.watching + [f"live:{short}"])
    h.wait_value(f"live:{short}", ANY, 20)
    time.sleep(2 * h.scale)
    said = [m for _, _, m in h.plugin.logs_since(mark) if short in m]
    refused = h.trace is not None and any(r.get("status") == 400 for r in http_since(
        h, h.trace.since(trace_mark), "GET", "/helix/users"))
    notes.append(f"3-character name {short}: value {h.plugin.values.get('live:' + short)!r}, "
                 f"{'Twitch refused the batch' if refused else 'no refused batch'}; log: {said[0] if said else '-'}")
    others = [n for n in chosen if h.plugin.values.get(f"live:{n}") == NOTHING]
    if others:
        return FAIL, "these lost their values too: " + ", ".join(others)
    return PASS, "; ".join(notes)


def check_b9(h):
    values = ["live:me", "title:me", "followers:me", "last_follower:me"]
    aside = h.tokens_path + ".harness-aside"
    h.plugin.stop()
    os.replace(h.tokens_path, aside)
    try:
        trace_mark = h.trace.last_seq() if h.trace else 0
        h.start_plugin()
        h.do(Watch(*values))
        for v in values:
            h.do(Value(v, PH))
        time.sleep(5 * h.scale)
        if h.trace is not None:
            calls = [r for r in h.trace.since(trace_mark) if r.get("kind") == "http" and "/helix/" in r.get("path", "")]
            if calls:
                return FAIL, f"{len(calls)} Helix call(s) while signed out"
            return PASS, "no Helix calls while signed out"
        return PASS, "every value showed the placeholder"
    finally:
        h.plugin.stop()
        if os.path.exists(aside):
            os.replace(aside, h.tokens_path)
        h.start_plugin()
        h.watch([])


def check_d6(h):
    state = h.plugin_state()
    if not state.get("last_clip_id"):
        h.do(Run("open-last-clip", expect=Warns(r"^no clip made from the deck yet$")))
        return None
    h.do(Run("open-last-clip", expect=OK()))
    h.do(Run("open-last-clip", editor=True, expect=OK()))
    h.do(Ask("Did the clip open, and then the clip editor?", optional=True))
    return None


def body_check(method, path, pick, want, label):
    """The last request of this kind since the case started carried this."""
    def check(h, records):
        sent = twitch_trace.http_records(records, method, path)
        if not sent:
            return FAIL, f"no {method} {path} in the trace"
        got = pick(sent[-1].get("request_body") or {}, sent[-1])
        return (PASS if got == want else FAIL), f"{label}: {got!r}"
    return check


def saved_order(h, records):
    stamp = h.stamp
    sent = [(r.get("request_body") or {}).get("message", "") for r in
            twitch_trace.http_records(records, "POST", "/helix/chat/messages")]
    cycle = [m.rsplit(" ", 1)[-1] for m in sent if stamp in m][:4]
    return (PASS if cycle == ["1", "2", "3", "1"] else FAIL), f"messages in order: {cycle}"


def check_e6(h):
    answers = []
    for i in range(25):
        _, _, extra = h.do(Run("chat", message=f"PerritoDeck test burst {h.stamp} {i + 1}", expect=Record()))
        answers.append(extra.get("answer") or {})
        time.sleep(1.2 * h.scale)
    first = next((i for i, a in enumerate(answers) if not a.get("ok")), None)
    h.do(Run("chat-color", color="blue_violet", expect=OK()))
    if first is None:
        return RECORDED, "Twitch took all 25"
    return RECORDED, f"Twitch refused from message {first + 1}: {answers[first].get('message')}"


def check_g7(h):
    _, _, extra = h.do(Run("vip", user="{V}", change="toggle", expect=Record()))
    if (extra.get("answer") or {}).get("ok"):
        h.do(Run("vip", user="{V}", change="toggle", expect=OK()))
        h.do(Ask("Did V get the VIP badge, and lose it again on the second press?"))
    return None


G7_ACTIONS = [Run("vip", user="{V}", change="toggle"), Run("vip", user="{V}", change="toggle")]


def check_f1_delay(h):
    """F1's "record whether delay shows without a moderator id": the harness's own
    GET /chat/settings without moderator_id, while the delay is on."""
    me = (h.tokens() or {}).get("user_id")
    status, answer = h.own_helix("GET", "/chat/settings", {"broadcaster_id": me})
    if status != 200:
        return RECORDED, f"GET /chat/settings without moderator_id: Twitch said {status}: {message_of(answer)}"
    data = (answer.get("data") or [{}])[0]
    if "non_moderator_chat_delay" in data:
        return RECORDED, (f"without moderator_id, Twitch shows the delay: non_moderator_chat_delay "
                          f"{data.get('non_moderator_chat_delay')!r}")
    return RECORDED, "without moderator_id, Twitch's answer leaves non_moderator_chat_delay out"


def ws_open(h, records):
    """An EventSub session is open now (read over the whole trace: plugin starts and stops,
    connections and the plugin closing them)."""
    open_, session = False, None
    for r in records:
        if r.get("kind") in ("start", "stop"):
            open_ = False
        elif r.get("kind") == "ws" and r.get("event") == "connect":
            open_, session = True, r.get("session")
        elif r.get("kind") == "ws" and r.get("event") == "close":
            open_ = False
    return (PASS, f"EventSub session {session} open") if open_ else (FAIL, "no EventSub session open")


def ws_connected(h, records):
    connects = [r for r in records if r.get("kind") == "ws" and r.get("event") == "connect"]
    return (PASS, f"EventSub session {connects[-1].get('session')}") if connects else (FAIL, "no EventSub connection")


def ws_closed(h, records):
    closes = [r for r in records if r.get("kind") == "ws" and r.get("event") == "close"]
    return (PASS, "the plugin closed its EventSub session") if closes else (FAIL, "the EventSub session is still open")


def no_new_connection(h, records):
    connects = [r for r in records if r.get("kind") == "ws" and r.get("event") == "connect"]
    return (FAIL, f"{len(connects)} new connection(s)") if connects else (PASS, "no new EventSub connection")


def subscribed(*types):
    def check(h, records):
        posts = twitch_trace.http_records(records, "POST", "/helix/eventsub/subscriptions")
        got = {}
        for p in posts:
            body = p.get("response_body") or {}
            data = (body.get("data") or [{}])[0] if isinstance(body, dict) else {}
            got[(p.get("request_body") or {}).get("type")] = (p.get("status"), data.get("status"))
        missing = [t for t in types if got.get(t, (None,))[0] != 202]
        detail = ", ".join(f"{t}: {got.get(t, ('not sent',))[0]} {got.get(t, (None, ''))[1] or ''}".strip() for t in types)
        return (FAIL if missing else PASS), detail
    return check


def l4_record(h, records):
    me = (h.tokens() or {}).get("user_id")
    posts = [p for p in twitch_trace.http_records(records, "POST", "/helix/eventsub/subscriptions")
             if (p.get("request_body") or {}).get("type") == "stream.online"
             and ((p.get("request_body") or {}).get("condition") or {}).get("broadcaster_user_id") != me]
    if not posts:
        return FAIL, "no stream.online subscription for the other channel yet"
    body = posts[-1].get("response_body") or {}
    return RECORDED, f"Twitch answered {posts[-1].get('status')}: {message_of(body) or 'subscribed'}"


def skip_when_zero(option):
    """A step that skips the case when a --...-minutes option is 0, as the README says."""
    def check(h):
        if not getattr(h.args, option):
            raise SkipCase(f"--{option.replace('_', '-')} 0")
        return DONE, f"--{option.replace('_', '-')} {getattr(h.args, option):g}"
    return check


def l10_rhythm(h, records):
    posts = [p for p in twitch_trace.http_records(records, "POST", "/helix/eventsub/subscriptions")
             if (p.get("request_body") or {}).get("type") == "channel.follow"]
    connects = [r for r in records if r.get("kind") == "ws" and r.get("event") == "connect"]
    gaps = [round(b["t"] - a["t"]) for a, b in zip(posts, posts[1:])]
    closed_4003 = any("4003" in m for _, _, m in h.plugin.logs_since(h.case_mark))
    detail = (f"{len(posts)} tries, {len(connects)} connection(s); seconds between tries: {gaps}; "
              f"Twitch's 4003 close {'seen' if closed_4003 else 'not seen'}")
    if not posts:
        return FAIL, "no subscription tried"
    if any(p.get("status") == 202 for p in posts):
        return FAIL, "Twitch took the subscription, so nothing was retried: " + detail
    expected = [15 * 2 ** i for i in range(len(gaps))]
    off = [g for g, e in zip(gaps, expected) if not (0.6 * min(e, 600) <= g <= 1.6 * min(e, 600) + 5)]
    if off or len(connects) > len(posts) + 1:
        return FAIL, detail
    return PASS, detail


def check_g5(h, records):
    bans = twitch_trace.http_records(records, "POST", "/helix/moderation/bans")
    if not bans:
        return FAIL, "no ban in the trace"
    data = (bans[-1].get("request_body") or {}).get("data") or {}
    return (PASS if data.get("duration") == 600 else FAIL), f"the ban sent duration {data.get('duration')!r}"


def expire_tokens(h):
    tokens = h.tokens()
    if not tokens:
        raise SkipCase("not signed in")
    tokens["expires_at"] = time.time() - 60
    write_json_atomically(h.tokens_path, tokens)


def check_a4(h):
    h.await_start_check()  # first: the check at start may refresh the token
    before = token_key((h.tokens() or {}).get("access_token"))
    mark = h.plugin.mark()
    _, _, extra = h.do(Run("sign-in", expect=Warns(r"enter")))
    code = (extra.get("answer") or {}).get("message", "")
    person_step(h, Do(f"Open the page ({code}) and press Deny, as {h.T}"))
    login, error, note = settle_sign_in(h, mark, 120)
    if login:
        return FAIL, f"approved instead of denied: {note or f'signed in as {login}'}"
    if error is None:
        return FAIL, f"no 'Twitch refused the sign-in' in the log: {note}"
    if not error.startswith("Twitch refused the sign-in"):
        return FAIL, f"the sign-in ended another way: {error}"
    after = token_key((h.tokens() or {}).get("access_token"))
    return (PASS if before == after else FAIL), f"{error}; the earlier sign-in {'kept' if before == after else 'changed'}"


def check_a5(h):
    h.await_start_check()
    mark = h.plugin.mark()
    if h.fake is not None:  # the fake approves nothing until the second code is out
        h.fake.approve_next_sign_in()
        h.fake.state["pending_polls"] = 10 ** 6
    h.do(Run("sign-in", expect=Warns(r"enter")))
    _, _, extra = h.do(Run("sign-in", expect=Warns(r"enter")))
    h.do(Log(r"^Twitch sign-in: replaced by a new sign-in$", since="previous"))
    person_step(h, Do(f"Approve only the second code ({(extra.get('answer') or {}).get('message', '')}), as {h.T}",
                      fake=lambda h: h.fake.state.update(device_polls=0, pending_polls=1)))
    login, error, note = settle_sign_in(h, mark, h.args.sign_in_minutes * 60)
    if login == h.T:
        return PASS, f"signed in as {login}"
    return FAIL, note or f"the sign-in didn't work: {error}"


def check_a9(h):
    old = token_key((h.tokens() or {}).get("refresh_token"))
    h.plugin.stop()
    expire_tokens(h)
    h.start_plugin()
    h.do(Log(r"^Twitch sign-in refreshed$", timeout=25, since="previous"))
    deadline = time.monotonic() + 5  # it logs that just before it saves the new tokens
    while token_key((h.tokens() or {}).get("refresh_token")) == old and time.monotonic() < deadline:
        time.sleep(0.1)
    tokens = h.tokens() or {}
    changed = token_key(tokens.get("refresh_token")) != old
    expires_in = round(float(tokens.get("expires_at") or 0) - time.time())
    h.do(Watch("live:me"))
    h.do(Value("live:me", ON_OFF))
    return (PASS if changed else FAIL), (f"refresh_token {'changed' if changed else 'unchanged'}; Twitch's "
                                         f"expires_in was about {expires_in} s")


def check_a10(h):
    colours = ["blue", "green", "coral", "dodger_blue", "blue_violet"]
    for colour in colours:
        h.allow("chat-color", {"color": colour})
    expire_tokens(h)
    mark, trace_mark = h.plugin.mark(), (h.trace.last_seq() if h.trace else 0)
    answers = [None] * len(colours)

    def press(i):
        answers[i] = h.plugin.run("chat-color", {"color": colours[i]})

    threads = [threading.Thread(target=press, args=(i,)) for i in range(len(colours))]
    for t in threads:
        t.start()
    for t in threads:
        t.join(15)
    refreshed = [m for _, _, m in h.plugin.logs_since(mark) if m == "Twitch sign-in refreshed"]
    failed = [f"{colours[i]}: {a.message if a else 'no answer'}" for i, a in enumerate(answers) if not (a and a.ok)]
    detail = f"{len(refreshed)} refresh(es) logged"
    if h.trace is not None:
        posts = [r for r in http_since(h, h.trace.since(trace_mark), "POST", "/oauth2/token")
                 if twitch_trace.grant(r) == "refresh_token"]
        detail += f", {len(posts)} in the trace"
        if len(posts) != 1:
            return FAIL, detail
    if failed or len(refreshed) != 1:
        return FAIL, detail + ("; failed: " + ", ".join(failed) if failed else "")
    return PASS, detail + "; all 5 keys worked"


def revoke_then_check(h, records):
    tokens_gone = not os.path.exists(h.tokens_path)
    revokes = twitch_trace.http_records(records, "POST", "/oauth2/revoke")
    detail = f"tokens.json {'gone' if tokens_gone else 'still there'}; revoke: {[r.get('status') for r in revokes]}"
    return (PASS if tokens_gone and revokes and revokes[-1].get("status") == 200 else FAIL), detail


def refresh_then_revoke(h, records):
    calls = [("refresh" if twitch_trace.grant(r) == "refresh_token" else "revoke")
             for r in records if r.get("kind") == "http" and r.get("path") in ("/oauth2/token", "/oauth2/revoke")
             and (r.get("path") == "/oauth2/revoke" or twitch_trace.grant(r) == "refresh_token")]
    return (PASS if calls[-2:] == ["refresh", "revoke"] else FAIL), f"calls: {calls}"


def check_a12(h):
    h.do(Watch("live:me", "last_raid:me"))
    h.do(TraceCheck(ws_open, "an EventSub session open", timeout=15, since="all"))
    h.do(Run("sign-out", expect=OK()))
    h.do(TraceCheck(revoke_then_check, "POST /oauth2/revoke, tokens.json gone", since="previous"))
    if h.trace is None:
        h.do(Custom(lambda h: (PASS if not os.path.exists(h.tokens_path) else FAIL, "tokens.json gone"),
                    "tokens.json gone"))
    h.do(Value("live:me", PH))
    h.do(TraceCheck(ws_closed, "the EventSub session closed", timeout=10))
    h.do(Ask("On twitch.tv as T, Settings, Connections: is the app gone from the list?"))
    h.do(SignIn())
    h.do(Watch())  # nothing in the background may refresh the expired token first
    h.do(TraceCheck(lambda h, r: (PASS, "no session") if ws_open(h, r)[0] == FAIL else (FAIL, "a session is open"),
                    "no EventSub session", timeout=10, since="all"))
    expire_tokens(h)
    _, _, extra = h.do(Run("sign-out", expect=OK()))
    h.do(TraceCheck(refresh_then_revoke, "a refresh, then the revoke", since="previous"))
    h.do(SignIn())
    seconds = (extra.get("answer") or {}).get("seconds")
    return RECORDED, f"sign-out with an expired token took {seconds} s (the host allows 10 s)"


def check_a14(h):
    before = h.plugin.values.get("last_raid:me")
    h.do(Restart())
    h.do(Log(r"^the saved Twitch sign-in is for {T}$", timeout=25, since="previous"))
    h.do(Value("session_follows:me", Exactly("0")))
    h.do(Value("last_raid:me", Exactly(before) if before is not None else ANY))
    return None


def check_a15(h):
    if h.trace is None:
        raise SkipCase("needs the trace, to see the approval arrive")
    h.await_start_check()
    mark, trace_mark = h.plugin.mark(), h.trace.last_seq()
    _, _, extra = h.do(Run("sign-in", expect=Warns(r"enter")))
    h.say(f"  Approve this code ({(extra.get('answer') or {}).get('message', '')}) as {h.T}; the harness presses "
          "Sign out the moment Twitch answers.")
    deadline = time.monotonic() + h.args.sign_in_minutes * 60
    while time.monotonic() < deadline:
        approved = [r for r in http_since(h, h.trace.since(trace_mark), "POST", "/oauth2/token")
                    if r.get("status") == 200 and twitch_trace.grant(r) != "refresh_token"]
        if approved:
            break
        time.sleep(0.02)
    else:  # cancel the attempt, so a late approval can't sign anyone in mid-run
        return FAIL, f"no approval arrived; {h.sign_out_now()}"
    h.do(Run("sign-out", expect=OK()))
    time.sleep(3)
    lines = [m for _, _, m in h.plugin.logs_since(mark)]
    signed_in_at = next((i for i, m in enumerate(lines) if m.startswith("signed in to Twitch as")), None)
    signed_out_at = next((i for i, m in enumerate(lines) if m == "signed out of Twitch"), None)
    if signed_in_at is not None and (signed_out_at is None or signed_in_at < signed_out_at):
        return RECORDED, "inconclusive: the approval was saved before the sign-out arrived; run A15 again"
    if os.path.exists(h.tokens_path) or (signed_in_at is not None and signed_in_at > (signed_out_at or 0)):
        return FAIL, "signed in again after the sign-out"
    return PASS, "stayed signed out; no tokens.json"


def soak(h):
    minutes = h.args.soak_minutes
    if not minutes:
        raise SkipCase("--soak-minutes 0")
    h.say(f"  M1: soaking for {minutes:g} minutes...")
    time.sleep(minutes * 60)
    return DONE, f"soaked {minutes:g} minutes"


def soak_verdict(minutes):
    def check(h):
        logs = [m for _, _, m in h.plugin.logs_since(h.case_mark)]
        bad = [m for m in logs if "EventSub connection:" in m]
        detail = [f"{len(bad)} 'EventSub connection:' warning(s)"]
        status = PASS if not bad else FAIL
        if h.trace is not None:
            s = twitch_trace.analyse(h.trace.since(h.case_trace))
            detail.append(f"{s['eventsub_connects']} connection(s), {s['eventsub_reconnect_messages']} "
                          f"session_reconnect(s); busiest minute {s['helix_busiest_minute']} Helix calls; "
                          f"{s['validations']} validation(s), gaps {s['validation_gaps_minutes']} min")
            if s["eventsub_connects"] > 1 + s["eventsub_reconnect_messages"] or s["helix_busiest_minute"] >= 800:
                status = FAIL
            if minutes >= 61 and s["validations"] < int(minutes // 60):
                status = FAIL
        return status, "; ".join(detail)
    return check


def check_a8(h):
    minutes = h.args.validate_minutes
    if not minutes:
        raise SkipCase("--validate-minutes 0")
    starts = []
    # A process of its own: the one running may have started (and been validated) in an
    # earlier case.
    h.do(Restart())
    starts.append(time.time())
    h.do(Watch("live:me"))
    h.say(f"  A8: one value key for {minutes:g} minutes...")
    time.sleep(minutes * 60)
    h.do(Restart())
    starts.append(time.time())
    h.do(Watch())
    h.say(f"  A8: no value keys for {minutes:g} minutes...")
    time.sleep(minutes * 60)
    told = [m for _, _, m in h.plugin.logs_since(h.case_mark) if m == START_LINE + h.T]
    if h.trace is None:
        return RECORDED, f"{len(told)} '{START_LINE}...' line(s); the validations need the trace"
    validations = [r["t"] for r in http_since(h, h.trace.since(h.case_trace), "GET", "/oauth2/validate")]
    phases = []
    for i, start in enumerate(starts):
        end = starts[i + 1] if i + 1 < len(starts) else time.time()
        phase = [round((t - start) / 60, 1) for t in validations if start - 5 <= t < end]
        phases.append(phase)
    ok = all(p and p[0] < 1.5 and (minutes < 61 or any(58 <= x <= 63 for x in p)) for p in phases)
    return (PASS if ok and len(told) >= 2 else FAIL), (f"minutes of each validation after each start: {phases}; "
                                                       f"{len(told)} '{START_LINE}{h.T}' line(s)")


def check_m5(h):
    """Use up T's rate limit with reads of its own account (16 at a time: Twitch refills
    the bucket faster than one request at a time empties it), then press a key."""
    me = (h.tokens() or {}).get("user_id")
    low = threading.Event()
    seen = {"sent": 0, "remaining": None}
    lock = threading.Lock()

    def read():
        while not low.is_set():
            with lock:
                if seen["sent"] >= 1500:
                    return
                seen["sent"] += 1
            try:
                status, _, headers = h.own_helix("GET", "/users", {"id": me}, with_headers=True)
            except SkipCase:
                return
            try:
                remaining = int(headers.get("Ratelimit-Remaining"))
            except (TypeError, ValueError, AttributeError):
                remaining = None
            with lock:
                seen["remaining"] = remaining
            if status == 429 or (remaining is not None and remaining <= 2):
                low.set()

    workers = [threading.Thread(target=read, daemon=True) for _ in range(16)]
    for w in workers:
        w.start()
    for w in workers:
        w.join(120)
    if not low.is_set():
        return FAIL, f"the bucket didn't run low after {seen['sent']} reads (Ratelimit-Remaining {seen['remaining']})"
    h.do(Run("chat-color", color="blue", expect=Record()))
    h.do(Run("chat-color", color="blue", expect=Fails(r"Twitch asked to slow down; try again in \d+ s")))
    time.sleep(65)
    h.do(Run("chat-color", color="blue", expect=OK()))
    return RECORDED, f"{seen['sent']} reads emptied the bucket"


def restore_channel(h):
    saved = h.saved
    title, category, tags = saved.get("title:me"), saved.get("category:me"), saved.get("tags:me")
    settings = {}
    if title and title != NOTHING:
        settings["title"] = title
    if category and category != NOTHING:
        settings["category"] = category
    if settings:
        h.do(Run("set-info", settings=settings, expect=Record()))
    if tags is not None:
        h.do(Run("tags", mode="set", tags="" if tags == NOTHING else tags, expect=Record()))
    return None


def restore_actions():
    """What restore_channel may run, for the yes prompt (the values are those it saw)."""
    return [Run("set-info", title=lambda h: h.saved.get("title:me"), category=lambda h: h.saved.get("category:me")),
            Run("tags", mode="set", tags=lambda h: h.saved.get("tags:me"))]


def remember(*value_ids):
    def fn(h):
        for vid in value_ids:
            h.wait_value(vid, ANY, 20)
            h.saved[vid] = h.plugin.values.get(vid)
        return DONE, "remembered " + ", ".join(f"{v}={h.saved.get(v)!r}" for v in value_ids)
    return fn


def bar(n):
    return "PerritoDeck test " + "x" * (n - 17)


SAVED = "PerritoDeck test {STAMP} 1 | PerritoDeck test {STAMP} 2 | PerritoDeck test {STAMP} 3"
LABELS = ("politics", "drugs", "gambling", "profanity", "sexual", "violence")


def label_steps():
    steps = []
    for label in LABELS:
        for state in ("toggle", "toggle", "on", "off"):
            steps.append(Run("label", settings={"label": label, "state": state}))
    default = next(f.get("default") for a in manifest()["actions"] if a["id"] == "label"
                   for f in a.get("fields", []) if f["id"] == "label")
    steps += [Run("label", settings={"label": default, "state": "on"}),
              Run("label", settings={"label": default, "state": "off"})]
    return steps


def chat_mode_steps():
    steps = []
    for mode in ("emote", "followers", "slow", "unique", "delay"):
        steps += [Run("chat-mode", mode=mode, state=s) for s in ("toggle", "toggle", "on", "off")]
    steps += [Run("chat-mode", mode="subscribers", state="on", expect=Record()),
              Run("chat-mode", mode="subscribers", state="off", expect=Record())]
    steps += [Run("chat-mode", mode="slow", state="on", duration=d) for d in ("3", "120")]
    steps += [Run("chat-mode", mode="followers", state="on", duration=d) for d in ("0", "10")]
    steps += [Run("chat-mode", mode="delay", state="on", duration=d) for d in ("2", "4", "6")]
    steps.append(Custom(check_f1_delay, "whether Twitch shows the delay without a moderator id"))
    steps += [Run("chat-mode", mode=m, state="off") for m in ("slow", "followers", "delay")]
    steps += [Run("chat-mode", mode="slow", state="on", duration="500", expect=Fails(r"must be 3 to 120")),
              Run("chat-mode", mode="delay", state="on", duration="3", expect=Fails(r"2, 4 or 6")),
              Run("chat-mode", mode="followers", state="on", duration="200000", expect=Fails(r"must be 0 to 129600")),
              Run("chat-mode", mode="loud", expect=Fails(r"the chat mode is"))]
    return steps


def cases():
    """Section 3 of docs/testing/twitch.md, in the order of its "Order" list."""
    title = "PerritoDeck test {STAMP}"
    return [
        # 1. No account involved.
        Case("A1", 1, "Empty client ID, press Sign in", AUTO, PRIVATE, signed_in=False, fake=True, steps=[
            Settings(client_id=""),
            Run("sign-in", expect=Fails(r"^set your Twitch app's client ID on the Plugins page$"))]),
        Case("A2", 1, "A random 30-character client ID, press Sign in", AUTO, PRIVATE, signed_in=False, fake=True,
             steps=[Settings(client_id=lambda h: random_client_id()),
                    Run("sign-in", expect=Fails(r"Twitch doesn't know that client ID \(.*\); check it, and that "
                                                r"the app is \"Public\""))]),
        # 2. Sign in as T.
        Case("A3", 2, "Sign in as T with every group ticked", SEMI, PRIVATE, signed_in=False, fake=True, steps=[
            SignIn(reuse=True),
            Custom(check_tokens_file, "tokens.json: login T, every group's scopes"),
            Custom(check_tokens_private, "tokens.json readable by the user only"),
            Custom(check_no_token_leak, "no token in the log, values, stderr or trace"),
            Ask("Did the consent page ask for every group's scopes (27), without "
                "moderator:read:automod_settings? Note how Twitch listed them.", optional=True)]),
        Case("A7", 2, "Polls and Moderation off: fewer scopes, and clear advice", SEMI, MODERATION,
             needs={"viewer"}, fake=True, confirm_note="the plugin should refuse both before calling Twitch", steps=[
                 Settings(group_polls=False, group_moderation=False),
                 SignIn(),
                 Run("poll-start", title="PerritoDeck test poll", choices="Yes, No",
                     expect=Fails(r"sign in again with Polls and predictions turned on in the plugin's settings")),
                 Run("ban", user="{V}", duration="1",
                     expect=Fails(r"sign in again with Moderation turned on in the plugin's settings")),
                 Settings(group_polls=True),
                 Run("poll-start", title="PerritoDeck test poll", choices="Yes, No",
                     expect=Fails(r"the sign-in doesn't allow that \(channel:manage:polls\)")),
                 TraceCheck(no_calls(("POST", "/helix/polls"), ("POST", "/helix/moderation/bans")),
                            "no Helix call for them"),
                 Ask("Did the consent page list fewer scopes than in A3?", optional=True),
                 Baseline(),
                 SignIn()]),
        Case("A13", 2, "Change the client ID while signed in", AUTO, PRIVATE, fake=True, steps=[
            Settings(client_id=lambda h: random_client_id()),
            Run("chat-color", color="blue", expect=Fails(r'^not signed in: press "Sign in to Twitch" first$'))]),
        # 3. Read-only values.
        Case("B1", 3, "live, viewers, uptime: me, a public channel, a made-up name", AUTO, READ, fake=True, steps=[
            Watch("live:me", "viewers:me", "uptime:me", "live:{P}", "viewers:{P}", "uptime:{P}",
                  "live:{U}", "viewers:{U}", "uptime:{U}"),
            Value("live:me", ON_OFF), Value("viewers:me", DIGITS), Value("uptime:me", UPTIME),
            Custom(live_consistent("me"), "off / 0 / offline, or on / N / Xm"),
            Value("live:{P}", ON_OFF), Value("viewers:{P}", DIGITS), Value("uptime:{P}", UPTIME),
            Value("live:{U}", PH), Value("viewers:{U}", PH), Value("uptime:{U}", PH),
            Log(r"^channel {U}: Twitch has no channel with that name$", count=1, settle=25),
            Ask("Do {P}'s live status and viewers match twitch.tv?", optional=True),
            # The plan's "then live" half: a person goes live (--check-by-eye).
            Do("B1 with T live: go live on T", optional=True),
            Value("live:me", Exactly("on"), timeout=120, optional=True),
            Value("uptime:me", r"\d+m|\d+h \d\dm", timeout=60, optional=True),
            Custom(live_consistent("me"), "on / N / Xm", optional=True),
            Do("Stop the stream on T", optional=True),
            Value("live:me", Exactly("off"), timeout=120, optional=True)]),
        Case("B2", 3, "title, category, tags: me and a public channel", AUTO, READ, fake=True, steps=[
            Watch("title:me", "category:me", "tags:me", "title:{P}", "category:{P}", "tags:{P}"),
            Value("title:me", ANY), Value("category:me", ANY), Value("tags:me", ANY),
            Value("title:{P}", ANY), Value("category:{P}", ANY), Value("tags:{P}", ANY),
            Do("Change T's title on the dashboard, with an emoji and an accent in it", optional=True),
            Value("title:me", ANY, timeout=45, after="previous", optional=True),
            Ask("Does the key show the new title as written, emoji and accents included?", optional=True)]),
        Case("B3", 3, "followers: me and a public channel", AUTO, READ, fake=True, steps=[
            Watch("followers:me", "followers:{P}"),
            Value("followers:me", DIGITS), Value("followers:{P}", DIGITS),
            Ask("Do the follower counts match twitch.tv?", optional=True)]),
        Case("B4", 3, "subscribers: me, then a public channel, without a refresh on every poll", AUTO, READ,
             fake=True, steps=[
                 Watch("subscribers:me", "subscribers:{P}"),
                 Value("subscribers:me", ANY, record=True),
                 Value("subscribers:{P}", PH),
                 Log(r"^subscribers of {P}: Twitch said \d{3}"),
                 Custom(no_refresh_while_polling("subscribers"), "no refresh over two polls")]),
        Case("B5", 3, "chatters: me, and a channel T doesn't moderate", AUTO, READ, fake=True, steps=[
            Watch("chatters:me", "chatters:{P}"),
            Value("chatters:me", DIGITS), Value("chatters:{P}", PH),
            Log(r"^chatters of {P}: Twitch said 403")]),
        Case("B6", 3, "chat_mode for every mode on me; slow and delay on a public channel", AUTO, READ, fake=True,
             steps=[
                 Watch(*[f"chat_mode:{m}" for m in ("emote", "followers", "slow", "subscribers", "unique", "delay")],
                       "chat_mode:slow@{P}", "chat_mode:delay@{P}"),
                 *[Value(f"chat_mode:{m}", ON_OFF) for m in ("emote", "followers", "slow", "subscribers", "unique",
                                                            "delay")],
                 Value("chat_mode:slow@{P}", ON_OFF),
                 Value("chat_mode:delay@{P}", PH, fake_note="the fake shows the delay to anyone who sends moderator_id"),
                 Ask("Do the modes match T's dashboard?", optional=True)]),
        Case("B7", 3, "Ad values on T", AUTO, READ, fake=True, steps=[
            Watch("next_ad:me", "ad_snoozes:me", "preroll_free:me", "ad_live:me"),
            Wait(8),
            *[Value(f"{v}:me", ANY, record=True) for v in ("next_ad", "ad_snoozes", "preroll_free", "ad_live")],
            Log(r"^ads of me: ", timeout=5, record=True)]),
        Case("B8", 3, "Ten channels plus an invalid name; then a 3-character name", AUTO, READ, fake=True, steps=[
            Custom(check_b8, "only the invalid name loses its values")]),
        Case("B9", 3, "Signed out, with value keys showing", AUTO, READ, fake=True, steps=[
            Custom(check_b9, "placeholders and no Helix calls")]),
        # 4. Changes only T can see.
        Case("E4", 4, "Chat colour: a name, a hex colour, an invalid one", AUTO, PRIVATE, fake=True, steps=[
            Run("chat-color", color="Blue Violet"),
            Run("chat-color", color="#9146FF", expect=Record()),
            Run("chat-color", color="mauve", expect=Fails(r"Turbo or Prime"))]),
        Case("D1", 4, "Marker while T is offline", AUTO, PUBLIC, steps=[
            RequireLive(False, ask=False),
            Run("marker", description="PerritoDeck test (offline)", expect=Fails())]),
        Case("D6", 4, "Open the last clip", AUTO, PRIVATE, fake=True, steps=[
            Custom(check_d6, "open-last-clip", effect=PRIVATE)]),
        # 5. T goes live.
        Case("D2", 5, "Marker while live, with a 200-character note", SEMI, PUBLIC, fake=True, steps=[
            RequireLive(True),
            Run("marker", description=bar(200)),
            Value("last_marker", r"\d+:\d\d(:\d\d)?", after="previous"),
            TraceCheck(body_check("POST", "/helix/streams/markers", lambda b, r: len(b.get("description", "")), 140,
                                  "note length sent"), "the note cut to 140"),
            Ask("Is the marker in Video Producer, with the note cut to 140 characters?")]),
        Case("D3", 5, "Marker with Store past broadcasts off", SEMI, PUBLIC, steps=[
            Do("Turn off Store past broadcasts on T's dashboard (Settings, Stream)"),
            Run("marker", description="PerritoDeck test (no VODs)", expect=Record()),
            Do("Turn Store past broadcasts back on")]),
        Case("D4", 5, "Clip on T while live, opening the editor", SEMI, PUBLIC, steps=[
            RequireLive(True),
            Run("clip", open=True, expect=OK()),
            Value("last_clip", r"\d\d:\d\d", after="previous"),
            Ask("Did the clip editor open, and does the clip exist about 15 s later? Delete it afterwards.")]),
        Case("D5", 5, "Clip while offline; clip on R while R is live", SEMI, PUBLIC, needs={"raid_target"}, steps=[
            Guided("Optional first: with T offline, press a Clip key on T and note the error."),
            RequireLive(True, who="{R}"),
            Run("clip", channel="{R}", expect=OK()),
            Ask("Is there a new clip on R's channel? Delete it afterwards.")]),
        Case("C1", 5, "Title only, then category only", AUTO, PUBLIC, fake=True, steps=[
            Watch("title:me", "category:me", "tags:me"),
            Custom(remember("title:me", "category:me", "tags:me"), "remember the title, category and tags"),
            Run("set-info", title=title),
            Value("title:me", Exactly(title), timeout=15),
            Run("set-info", category="Just Chatting"),
            Value("category:me", Exactly("Just Chatting"), timeout=15),
            Custom(restore_channel, "put the title and category back", effect=PUBLIC, actions=restore_actions())]),
        Case("C2", 5, "Category in lower case, and a misspelt one", AUTO, PUBLIC, fake=True, steps=[
            Watch("title:me", "category:me", "tags:me"),
            Custom(remember("title:me", "category:me", "tags:me"), "remember the title, category and tags"),
            Run("set-info", category="just chatting", expect=Record()),
            Run("set-info", category="Mincraft",
                expect=Fails(r"^Twitch has no category called 'Mincraft'; use its exact name, as on twitch\.tv$")),
            Custom(restore_channel, "put the category back", effect=PUBLIC, actions=restore_actions())]),
        Case("C3", 5, "A 141-character title; every field empty", AUTO, PUBLIC, fake=True,
             confirm_note="the plugin should refuse both before calling Twitch", steps=[
            Run("set-info", title=bar(141), expect=Fails(r"titles are up to 140 characters")),
            Run("set-info", expect=Fails(r"^enter a title, a category or both$")),
            TraceCheck(no_calls(("PATCH", "/helix/channels")), "nothing sent")]),
        Case("C4", 5, "Presets 1-5: with tags, without, title only, category only, empty", AUTO, PUBLIC,
             fake=True, steps=[
                 Watch("title:me", "category:me", "tags:me"),
                 Custom(remember("title:me", "category:me", "tags:me"), "remember the title, category and tags"),
                 Settings(preset_1=title + " 1 | Just Chatting | PerritoDeck, Test",
                          preset_2=title + " 2 | Just Chatting", preset_3=title + " 3", preset_4=" | Just Chatting",
                          preset_5=""),
                 Run("preset", preset="1"),
                 TraceCheck(body_check("PATCH", "/helix/channels", lambda b, r: sorted(b),
                                       ["game_id", "tags", "title"], "fields sent"), "title, category and tags"),
                 Run("preset", preset="2"),
                 TraceCheck(body_check("PATCH", "/helix/channels", lambda b, r: sorted(b), ["game_id", "title"],
                                       "fields sent"), "title and category"),
                 Run("preset", preset="3"),
                 TraceCheck(body_check("PATCH", "/helix/channels", lambda b, r: sorted(b), ["title"], "fields sent"),
                            "title only"),
                 Run("preset", preset="4"),
                 TraceCheck(body_check("PATCH", "/helix/channels", lambda b, r: sorted(b), ["game_id"], "fields sent"),
                            "category only"),
                 Run("preset", preset="5", expect=Fails(r"^preset 5 is empty")),
                 Custom(restore_channel, "put the title, category and tags back", effect=PUBLIC,
                        actions=restore_actions())]),
        Case("C5", 5, "Tags: set, add, remove; invalid ones; Español; an empty set", AUTO, PUBLIC, fake=True, steps=[
            Watch("tags:me"),
            Custom(remember("tags:me"), "remember the tags"),
            Run("tags", mode="set", tags="PerritoDeck, Test"),
            Run("tags", mode="add", tags="Harness"),
            Run("tags", mode="remove", tags="test"),
            Value("tags:me", r"PerritoDeck, Harness|Harness, PerritoDeck", timeout=15),
            Run("tags", mode="set", tags="two words", expect=Fails(r"isn't a valid tag")),
            Run("tags", mode="set", tags="a-b", expect=Fails(r"isn't a valid tag")),
            Run("tags", mode="set", tags=",".join(f"t{i}" for i in range(11)), expect=Fails(r"up to 10 tags")),
            Run("tags", mode="set", tags="Español", expect=Record()),
            Run("tags", mode="set", tags=""),
            Value("tags:me", PH, timeout=15),
            Custom(restore_channel, "put the tags back", effect=PUBLIC, actions=restore_actions())]),
        Case("C6", 5, "Content labels: each one toggled, on and off; the picker's default", AUTO, PUBLIC, fake=True,
             confirm_note="each label ends turned off", steps=[
                 *label_steps(),
                 Ask("Did the dashboard show each label as it changed?", optional=True)]),
        Case("C7", 5, "Branded content: toggle, on, off", AUTO, PUBLIC, fake=True, steps=[
            Run("branded", state="toggle"), Run("branded", state="toggle"),
            Run("branded", state="on"), Run("branded", state="off"),
            Ask("Did the dashboard's branded content box follow?", optional=True)]),
        # 6. Public chat.
        Case("E1", 6, "Chat on T and on R; a 501-character message", SEMI, PUBLIC, needs={"raid_target"}, fake=True,
             steps=[
                 Run("chat", message="PerritoDeck test: hello from the test harness"),
                 Run("chat", message="PerritoDeck test: hello from the test harness", channel="{R}"),
                 Run("chat", message=bar(501)),
                 TraceCheck(body_check("POST", "/helix/chat/messages", lambda b, r: len(b.get("message", "")), 500,
                                       "length sent"), "the long one cut to 500"),
                 Ask("Did V see the messages in T's and R's chat, the long one cut to 500 characters?")]),
        Case("E2", 6, "A message Twitch holds back (R's blocked terms or AutoMod)", SEMI, PUBLIC,
             needs={"raid_target"}, steps=[
                 AskText("A term on R's blocked-terms list (Enter to skip):", "blocked"),
                 Run("chat", channel="{R}", message=lambda h: f"PerritoDeck test {h.answers['blocked']}",
                     expect=Fails(r"^Twitch didn't send it: "))]),
        Case("E3", 6, "Saved messages in order across a restart, then at random", SEMI, PUBLIC, fake=True, steps=[
            Run("chat-saved", messages=SAVED, order="cycle"),
            Restart(),
            Run("chat-saved", messages=SAVED, order="cycle"),
            Run("chat-saved", messages=SAVED, order="cycle"),
            Run("chat-saved", messages=SAVED, order="cycle"),
            TraceCheck(saved_order, "1, 2, 3, 1 across the restart"),
            *[Run("chat-saved", messages=SAVED, order="random") for _ in range(3)],
            Ask("Did the messages arrive 1, 2, 3, 1, and then in a varying order?")]),
        Case("E5", 6, "Announcements in every colour; in a channel where T isn't a moderator", SEMI, PUBLIC,
             needs={"raid_target"}, fake=True, steps=[
                 *[Run("announce", message=f"PerritoDeck test announcement ({c})", color=c)
                   for c in ("primary", "blue", "green", "orange", "purple")],
                 Run("announce", message="PerritoDeck test announcement", channel="{R}", expect=Record()),
                 Ask("Did five coloured announcements appear in T's chat?")]),
        Case("E6", 6, "25 chat messages in 30 s", AUTO, PUBLIC, fake=True,
             confirm_note="sends 25 chat messages to T's chat in 30 s", steps=[
                 Custom(check_e6, "when Twitch starts refusing", effect=PUBLIC,
                        actions=[Run("chat", message="PerritoDeck test burst {STAMP} 1 ... 25")])]),
        # 7. Chat settings on T.
        Case("F1", 7, "Chat modes: each mode, durations, out-of-range values", SEMI, MODERATION, fake=True,
             confirm_note="every mode ends turned off", steps=[
                 *chat_mode_steps(),
                 Ask("Did V's chat show each mode as it changed?")]),
        # F2 and G9 act in R's channel, where the harness never moderates: a person does them.
        Case("F2", 7, "Chat modes in R's channel", SEMI, MODERATION, needs={"raid_target"}, steps=[
            Guided("Press a chat-mode key with Channel = {R} (emote on, then off), first with T as {R}'s moderator, "
                   "then not. Expected: it works as moderator, and Twitch's 403 otherwise.", by_hand=["chat-mode"])]),
        Case("F3", 7, "Shield Mode: toggle, on, off", SEMI, MODERATION, fake=True, steps=[
            Run("shield", state="toggle"), Run("shield", state="toggle"),
            Run("shield", state="on"), Run("shield", state="off"),
            Ask("Did the dashboard show the Shield Mode banner, and then not?")]),
        Case("F4", 7, "AutoMod levels 0 to 4, then back", SEMI, MODERATION, fake=True, steps=[
            AskText("T's AutoMod level now, 0 to 4 (Enter for 1):", "automod", default="1"),
            *[Run("automod", level=str(n)) for n in range(5)],
            Run("automod", level=lambda h: h.answers["automod"]),
            Ask("Did the dashboard's moderation settings follow each level?")]),
        # 8. EventSub.
        Case("L1", 8, "Events on, a last_* key showing: one session, subscriptions within 10 s", AUTO, READ,
             fake=True, steps=[
                 Watch("last_follower:me", "last_raid:me"),
                 TraceCheck(ws_open, "an EventSub session open", timeout=15, since="all"),
                 TraceCheck(subscribed("channel.follow", "channel.raid"), "both subscriptions, enabled", timeout=12)]),
        Case("L2", 8, "V follows T", SEMI, READ, needs={"viewer"}, steps=[
            Watch("last_follower:me", "session_follows:me"),
            Value("session_follows:me", DIGITS),
            Do("As V, unfollow T if needed, then follow T"),
            Value("last_follower:me", r"(?i){V}", timeout=120),
            Value("session_follows:me", r"[1-9]\d*")]),
        Case("L3", 8, "T goes live, then offline: live flips within about 2 s", SEMI, READ, steps=[
            Watch("live:me", "last_raid:me"),
            Do("Go live on T"),
            Value("live:me", Exactly("on"), timeout=120),
            Do("Stop the stream on T"),
            Value("live:me", Exactly("off"), timeout=120)]),
        Case("L4", 8, "live:<R> with Events on", AUTO, READ, needs={"raid_target"}, fake=True, steps=[
            Watch("live:{R}"),
            TraceCheck(l4_record, "whether Twitch takes stream.online for another broadcaster", timeout=15)]),
        Case("L5", 8, "A follow subscription for a channel T doesn't moderate", AUTO, READ, steps=[
            Watch("last_follower:{P}", "last_raid:me"),
            Log(r"^event channel\.follow: ", count=1, settle=20),
            TraceCheck(subscribed("channel.raid"), "the others keep working", timeout=10)]),
        Case("L6", 8, "Subs, gifts, cheers, hype trains", MANUAL, READ, steps=[
            Guided("With the Twitch CLI's mock server (a download: the maintainer's OK first) or real subs and bits "
                   "on M: check last_subscriber, last_gift, last_cheer and hype_train map the fields correctly.")]),
        Case("L7", 8, "Events group off: no WebSocket, placeholders", AUTO, PRIVATE, fake=True, steps=[
            Watch("last_cheer:me"),
            TraceCheck(ws_open, "an EventSub session open", timeout=15, since="all"),
            Settings(group_events=False),
            TraceCheck(ws_closed, "the session closed", timeout=10, since="previous"),
            Wait(10),
            TraceCheck(no_new_connection, "no new session", since="previous"),
            Value("last_cheer:me", PH)]),
        Case("L8", 8, "Remove every event key: the session closes within about 5 s", AUTO, READ, fake=True, steps=[
            Watch("last_raid:me"),
            TraceCheck(ws_open, "an EventSub session open", timeout=15, since="all"),
            Watch(),
            TraceCheck(ws_closed, "the session closed", timeout=8, since="previous")]),
        Case("L9", 8, "EventSub blocked in the firewall", SEMI, READ, steps=[
            Guided("Block eventsub.wss.twitch.tv in the firewall (the harness never changes system settings); "
                   "last_follower should fall back to polling within 30 s, and EventSub retry from 2 s up to 60 s.")]),
        Case("L10", 8, "Only a refused subscription: retries back off, no session in between", AUTO, READ, steps=[
            Custom(skip_when_zero("l10_minutes"), "--l10-minutes isn't 0"),
            Watch("last_follower:{P}"),
            Custom(lambda h: (time.sleep(h.args.l10_minutes * 60) or (DONE, "waited")), "wait (--l10-minutes)"),
            TraceCheck(l10_rhythm, "15 s, 30 s, 1 min ... with one connection and one POST per try")]),
        # 9. Moderation on V.
        Case("G1", 9, "Warn V, with and without a reason", SEMI, MODERATION, needs={"viewer"}, fake=True, steps=[
            Run("warn", user="{V}", reason="PerritoDeck test warning"),
            Run("warn", user="{V}", expect=Fails(r"enter the reason")),
            Ask("Did V have to acknowledge the warning before chatting?")]),
        Case("G2", 9, "Time V out for a minute, then unban", SEMI, MODERATION, needs={"viewer"}, fake=True, steps=[
            Run("ban", user="{V}", duration="60", reason="PerritoDeck test"),
            Ask("Is V unable to chat?"),
            Run("unban", user="{V}"),
            Ask("Can V chat again?")]),
        Case("G3", 9, "Ban V with no end, then unban", SEMI, MODERATION, needs={"viewer"}, fake=True, steps=[
            Run("ban", user="{V}", duration="0", reason="PerritoDeck test"),
            Ask("Is V banned?"),
            Run("unban", user="{V}"),
            Ask("Is V unbanned?")]),
        Case("G4", 9, "Ban a made-up name, T itself, V twice; unban someone not banned", AUTO, MODERATION,
             needs={"viewer"}, fake=True, steps=[
                 Run("ban", user="{U}", duration="60", expect=Fails(r"^Twitch has no user called {U}$")),
                 Run("ban", user="{T}", duration="60", expect=Record()),
                 Run("ban", user="{V}", duration="60"),
                 Run("ban", user="{V}", duration="60", expect=Record()),
                 Run("unban", user="{V}"),
                 Run("unban", user="{V}", expect=Record())]),
        Case("G5", 9, "Ban V with no duration saved: a 10-minute timeout", AUTO, MODERATION, needs={"viewer"},
             fake=True, steps=[
                 Run("ban", user="{V}"),
                 TraceCheck(check_g5, "duration 600, not a ban with no end"),
                 Run("unban", user="{V}")]),
        Case("G6", 9, "Clear chat", SEMI, MODERATION, fake=True, steps=[
            Run("clear-chat"),
            Ask("Was every message removed from T's chat?")]),
        Case("G7", 9, "VIP toggle on V", SEMI, MODERATION, needs={"viewer"}, fake=True, steps=[
            Custom(check_g7, "VIP on, then off", effect=MODERATION, actions=G7_ACTIONS)]),
        Case("G8", 9, "Mod V: add, add again, remove; mod V while V is a VIP", SEMI, MODERATION, needs={"viewer"},
             fake=True, steps=[
                 Run("mod", user="{V}", change="add"),
                 Run("mod", user="{V}", change="add", expect=Record()),
                 Run("mod", user="{V}", change="remove"),
                 Run("vip", user="{V}", change="add", expect=Record()),
                 Run("mod", user="{V}", change="add", expect=Record()),
                 Run("mod", user="{V}", change="remove", expect=Record()),
                 Run("vip", user="{V}", change="remove", expect=Record())]),
        Case("G9", 9, "Ban V in R's channel", SEMI, MODERATION, needs={"viewer", "raid_target"}, steps=[
            Guided("Press a Ban key on {V} with Channel = {R} and a 1-minute timeout, first with T as {R}'s "
                   "moderator, then not; unban {V} there afterwards. Expected: it works as moderator, and Twitch's "
                   "403 otherwise.", by_hand=["ban", "unban"])]),
        # 10. Shoutouts, then raids.
        Case("H5", 10, "Shoutouts: R, R again, T itself; another action right after; while offline", SEMI, PUBLIC,
             needs={"raid_target"}, steps=[
                 RequireLive(True),
                 Run("shoutout", channel="{R}"),
                 Run("shoutout", channel="{R}", expect=Record()),
                 Run("shoutout", channel="{T}", expect=Record()),
                 Run("chat-color", color="blue_violet"),
                 Ask("Did the shoutout card appear in T's chat?"),
                 RequireLive(False),
                 Run("shoutout", channel="{R}", expect=Record())]),
        Case("H1", 10, "Raid R, then cancel within 90 s", SEMI, PUBLIC, needs={"raid_target"}, steps=[
            RequireLive(True), RequireLive(True, who="{R}"),
            Run("raid", channel="{R}"),
            Wait(10, scaled=False),
            Run("cancel-raid"),
            Ask("Did the raid countdown appear on T and then go, with R not raided?")]),
        Case("H2", 10, "Raid R and let it complete", SEMI, PUBLIC, needs={"raid_target"}, steps=[
            RequireLive(True), RequireLive(True, who="{R}"),
            Run("raid", channel="{R}"),
            Ask("Did R receive the raid (about 90 s)?")]),
        Case("H3", 10, "Raid errors: T offline, R offline, nothing to cancel", SEMI, PUBLIC, needs={"raid_target"},
             steps=[
                 Run("cancel-raid", expect=Record()),
                 Guided("Raid R while T is offline, and raid R while R is offline; note each error.",
                        by_hand=["raid"])]),
        Case("H4", 10, "R raids T", SEMI, READ, needs={"raid_target"}, steps=[
            Watch("last_raid:me"),
            Do("As R, raid T (from R's chat: /raid followed by T's name)"),
            Value("last_raid:me", r"(?i){R} \(\d+\)", timeout=180)]),
        # 11. Errors and the token's life.
        Case("A4", 11, "Sign in, then Deny on Twitch", SEMI, PRIVATE, steps=[
            Custom(check_a4, "Twitch refused the sign-in; the earlier one kept", effect=PRIVATE)]),
        Case("A5", 11, "Sign in twice; approve only the second code", SEMI, PRIVATE, fake=True, steps=[
            Custom(check_a5, "the first is replaced, the second works", effect=PRIVATE)]),
        Case("A6", 11, "The Confidential app", SEMI, PRIVATE, needs={"confidential"}, steps=[
            Settings(client_id=lambda h: h.args.confidential_client_id),
            Run("sign-in", expect=Record()),
            Guided("Try to approve the code with the Confidential app; note what the device and token requests "
                   "returned (the plugin's log) and whether the advice reached you."),
            Baseline()]),
        Case("A9", 11, "An expired token at start", SEMI, PRIVATE, fake=True, steps=[
            Custom(check_a9, "refreshed, with a new refresh token", effect=PRIVATE)]),
        Case("A10", 11, "An expired token and five keys at once", AUTO, PRIVATE, fake=True, steps=[
            Custom(check_a10, "exactly one refresh; all five work", effect=PRIVATE)]),
        Case("A11", 11, "The app disconnected on twitch.tv", MANUAL, PRIVATE, fake=True, steps=[
            Watch("live:me", "last_raid:me"),
            Value("live:me", ON_OFF),
            Do("On twitch.tv as T: Settings, Connections: disconnect your PerritoDeck test app",
               fake=lambda h: h.fake.end_sign_in()),
            Run("chat-color", color="blue", expect=Fails(r"Twitch ended the sign-in; press \"Sign in to Twitch\" again")),
            Value("live:me", PH, timeout=30),
            Log(r"Twitch stopped sending these", timeout=30, record=True)]),
        Case("A12", 11, "Sign out; then sign out with an expired token", SEMI, PRIVATE, fake=True, steps=[
            Custom(check_a12, "revoked, forgotten, closed", effect=PRIVATE)]),
        Case("A14", 11, "Restart while signed in", SEMI, PRIVATE, fake=True, steps=[
            Watch("last_raid:me", "session_follows:me"),
            Value("session_follows:me", DIGITS), Value("last_raid:me", ANY),
            Custom(check_a14, "still signed in; last_* back; session_* at 0", effect=PRIVATE)]),
        Case("A15", 11, "Sign out while the approval is on its way", SEMI, PRIVATE, steps=[
            Custom(check_a15, "stays signed out", effect=PRIVATE)]),
        Case("M2", 11, "The network gone for 2 minutes", SEMI, PRIVATE, steps=[
            Watch("live:me", "last_raid:me"),
            Do("Disconnect this computer from the network (leave the harness running)"),
            Run("chat-color", color="blue", expect=Fails(r"couldn't reach Twitch")),
            Log(r"couldn't reach Twitch", timeout=30),
            Wait(120, scaled=False),
            Do("Reconnect the network"),
            Value("live:me", ON_OFF, timeout=60),
            TraceCheck(ws_connected, "EventSub back", timeout=60, since="previous")]),
        Case("M3", 11, "The network gone while subscribing", SEMI, READ, steps=[
            Guided("Pull the network just after an EventSub reconnect, while it subscribes; the failed "
                   "subscription should recover on its own (at once, or after 15 s, 30 s, 1 min ...).")]),
        Case("M4", 11, "The plugin killed; the computer asleep", SEMI, PRIVATE, fake=True, steps=[
            Watch("last_raid:me", "live:me"),
            TraceCheck(ws_open, "an EventSub session open", timeout=15, since="all"),
            Restart(kill=True),
            Log(r"^the saved Twitch sign-in is for {T}$", timeout=25, since="previous"),
            TraceCheck(subscribed("channel.raid"), "subscribed again", timeout=15, since="previous"),
            Do("Put the computer to sleep for a minute and wake it", optional=True),
            Ask("After waking: still signed in, and last_raid still working?", optional=True)]),
        Case("M5", 11, "The rate limit used up on T", AUTO, PRIVATE, steps=[
            Custom(check_m5, "the plugin pauses and resumes", effect=PRIVATE)]),
        Case("M6", 11, "A throttled network and a multi-request action", MANUAL, READ, steps=[
            Guided("With a network throttling tool (the maintainer's OK first: it's a download), press a "
                   "multi-request action; note whether the key shows the host's timeout while it still completes.")]),
        Case("N1", 11, "Windows: the browser opens and stays open", SEMI, PRIVATE, needs={"windows"}, steps=[
            Ask("When you signed in, did the browser open twitch.tv/activate with the code?"),
            Restart(),
            Ask("Is that browser still open after the plugin restarted?")]),
        Case("N2", 11, "macOS and Linux: open / xdg-open; tokens.json mode 600", SEMI, PRIVATE, needs={"posix"},
             steps=[
                 Custom(check_tokens_private, "tokens.json mode 600"),
                 Ask("When you signed in, did the browser open twitch.tv/activate with the code?")]),
        Case("A8", 11, "Validation at start and hourly, with and without value keys", AUTO, PRIVATE, steps=[
            Custom(check_a8, "validated at each start and an hour later", effect=PRIVATE)]),
        Case("M1", 11, "A long idle soak", AUTO, READ, steps=[
            Watch("live:me", "viewers:me", "title:me", "followers:me", "last_follower:me", "last_raid:me",
                  "last_cheer:me"),
            Custom(soak, "soak (--soak-minutes)"),
            Custom(lambda h: soak_verdict(h.args.soak_minutes)(h), "no Ping reconnects; calls well under 800 a minute")]),
        # 12. Affiliate or Partner channels only.
        Case("J1", 12, "reward-create; reward values", AUTO, PUBLIC, steps=[
            Run("reward-create", title="PerritoDeck test", cost="1", expect=Record()),
            Watch("reward_enabled:PerritoDeck test"),
            Value("reward_enabled:PerritoDeck test", ANY, record=True)]),
        Case("J2", 12, "Create the test reward", SEMI, PUBLIC, steps=[
            Run("reward-create", title="PerritoDeck test", cost="1", prompt="Type anything", input=True),
            Watch("reward_enabled:PerritoDeck test"),
            Value("reward_enabled:PerritoDeck test", Exactly("on"))]),
        Case("J3", 12, "Pause, unpause, disable, enable, cost 5", AUTO, PUBLIC, steps=[
            Watch("reward_paused:PerritoDeck test", "reward_enabled:PerritoDeck test"),
            Run("reward", reward="PerritoDeck test", change="pause"),
            Value("reward_paused:PerritoDeck test", Exactly("on"), timeout=40),
            Run("reward", reward="PerritoDeck test", change="unpause"),
            Run("reward", reward="PerritoDeck test", change="disable"),
            Value("reward_enabled:PerritoDeck test", Exactly("off"), timeout=40),
            Run("reward", reward="PerritoDeck test", change="enable"),
            Run("reward", reward="PerritoDeck test", change="cost", cost="5")]),
        Case("J4", 12, "A reward made on twitch.tv", AUTO, PUBLIC, steps=[
            AskText("The title of a reward made on twitch.tv:", "web_reward"),
            Run("reward", reward=lambda h: h.answers["web_reward"], change="pause",
                expect=Fails(r"Twitch only lets the app that made a reward change it"))]),
        Case("J5", 12, "Redemptions: fulfil, then reject and refund", SEMI, PUBLIC, needs={"viewer"}, steps=[
            Watch("reward_pending:PerritoDeck test", "last_redemption:me"),
            Do("As V, redeem PerritoDeck test twice"),
            Value("reward_pending:PerritoDeck test", Exactly("2"), timeout=60),
            Run("redemption", reward="PerritoDeck test", outcome="fulfil"),
            Run("redemption", reward="PerritoDeck test", outcome="cancel"),
            Run("redemption", reward="PerritoDeck test", expect=Warns(r"^no redemptions waiting$"))]),
        Case("J6", 12, "A duplicate title; a 50-character title", AUTO, PUBLIC, steps=[
            Run("reward-create", title="PerritoDeck test", cost="1", expect=Record()),
            Run("reward-create", title=bar(50), cost="1"),
            TraceCheck(body_check("POST", "/helix/channel_points/custom_rewards", lambda b, r: len(b.get("title", "")),
                                  45, "title length sent"), "cut to 45")]),
        Case("J7", 12, "Remove the test rewards", SEMI, PUBLIC, steps=[
            Guided("Run py -3 plugins/tools/twitch_cleanup.py --match \"PerritoDeck test\" (a dry run), then again "
                   "with --broadcaster {T} --yes, and check the rewards are gone.")]),
        Case("K1", 12, "poll-start and prediction-start", AUTO, PUBLIC, steps=[
            Run("poll-start", title="PerritoDeck test poll", choices="Yes, No", duration="15", expect=Record()),
            Run("prediction-start", title="PerritoDeck test", outcomes="Yes, No", window="30", expect=Record())]),
        Case("K2", 12, "A 15 s poll with 3 choices; V votes", SEMI, PUBLIC, needs={"viewer"}, steps=[
            Watch("poll:me", "poll_leader:me", "poll_time:me"),
            Run("poll-start", title="PerritoDeck test poll", choices="A, B, C", duration="15"),
            Value("poll:me", Exactly("PerritoDeck test poll"), timeout=15),
            Do("As V, vote in the poll"),
            Value("poll_time:me", Exactly("ended"), timeout=40)]),
        Case("K3", 12, "End a poll; no poll; invalid polls", AUTO, PUBLIC, steps=[
            Run("poll-start", title="PerritoDeck test poll", choices="A, B", duration="60"),
            Run("poll-end", how="end"),
            Run("poll-end", expect=Warns(r"^no poll is running$")),
            Run("poll-start", title="PerritoDeck test poll", choices="A, B", duration="60"),
            Run("poll-end", how="archive"),
            Run("poll-start", title=bar(61), choices="A, B", expect=Fails(r"up to 60 characters")),
            Run("poll-start", title="PerritoDeck test", choices="A, " + "b" * 26, expect=Fails(r"choices up to 25")),
            Run("poll-start", title="PerritoDeck test", choices="A, B, C, D, E, F", expect=Fails(r"2 to 5 choices")),
            Run("poll-start", title="PerritoDeck test", choices="A, B", duration="14",
                expect=Fails(r"must be 15 to 1800"))]),
        Case("K4", 12, "Predictions: lock, resolve by name and number, cancel", SEMI, PUBLIC, needs={"viewer"},
             confirm_note="the last prediction is cancelled, which refunds", steps=[
                 Run("prediction-start", title="PerritoDeck test", outcomes="Yes, No", window="30"),
                 Do("As V, predict"),
                 Run("prediction-lock"),
                 Run("prediction-lock", expect=Warns(r"already locked")),
                 Run("prediction-resolve", outcome="yes"),
                 Run("prediction-start", title="PerritoDeck test", outcomes="A, B, C", window="30"),
                 Run("prediction-resolve", outcome="2"),
                 Run("prediction-start", title="PerritoDeck test", outcomes="A, B", window="30"),
                 Run("prediction-cancel"),
                 Ask("Were points paid to the winners, and refunded on the cancelled one?")]),
        Case("K5", 12, "Resolve while open; an outcome that doesn't exist", AUTO, PUBLIC,
             confirm_note="the prediction is cancelled at the end, which refunds", steps=[
                 Run("prediction-start", title="PerritoDeck test", outcomes="Yes, No", window="60"),
                 Run("prediction-resolve", outcome="Maybe", expect=Fails(r"has no outcome 'Maybe'")),
                 Run("prediction-resolve", outcome="Yes", expect=Record()),
                 Run("prediction-cancel", expect=Record())]),
        Case("I1", 12, "ad and snooze-ad on the signed-in channel", AUTO, PUBLIC, steps=[
            Run("ad", length="30", expect=Record()),
            Run("snooze-ad", expect=Record())]),
        Case("I4", 12, "An ad while offline", AUTO, PUBLIC, steps=[
            RequireLive(False, ask=False),
            Run("ad", length="30", expect=Fails(r"must be streaming live"))]),
        Case("I2", 12, "Snooze until the snoozes run out", SEMI, PUBLIC, steps=[
            RequireLive(True),
            Watch("next_ad:me", "ad_snoozes:me"),
            *[Run("snooze-ad", expect=Record()) for _ in range(4)],
            Run("chat-color", color="blue_violet")]),
        Case("I3", 12, "One 30 s ad, then a second at once", MANUAL, PUBLIC, steps=[
            RequireLive(True),
            Watch("ad_live:me"),
            Run("ad", length="30"),
            Value("ad_live:me", Exactly("on"), timeout=15),
            Run("ad", length="30", expect=Record())]),
    ]


# --- the harness -------------------------------------------------------------------------------


class Harness:
    def __init__(self, args, fake=None, input_fn=input, out=None, auto_yes=None, make_login=made_up_login):
        self.args = args
        self.fake = fake
        self.input_fn = input_fn
        self.out = out or sys.stdout
        self.auto_yes = (fake is not None) if auto_yes is None else (auto_yes and fake is not None)
        self.person = fake is None and not args.auto_only
        self.make_login = make_login
        self.scale = 0.2 if fake is not None else 1.0
        self.redactor = Redactor()
        # Device codes: shown on the console (the person needs them), kept out of the report.
        self.codes = Redactor()
        self.T = login_key(args.broadcaster)
        self.V = login_key(args.viewer) or None
        self.R = login_key(args.raid_target) or None
        self.P = login_key(args.public_channel)
        self.M = login_key(args.main_channel) or None
        self.client_id = args.client_id
        self.helix_url = fake.helix_url if fake is not None else HELIX_URL
        self.stamp = time.strftime("%H%M%S")
        self.baseline = {"client_id": self.client_id, **{f"group_{g}": True for g in
                                                         ("ads", "moderation", "points", "polls", "events")},
                         **{f"preset_{n}": "" for n in range(1, 6)}}
        self.settings = dict(self.baseline)
        self.folder = args.data_dir
        self.data_dir = plugin_data(self.folder)
        self.tokens_path = os.path.join(self.data_dir, "tokens.json")
        self.stderr_path = os.path.join(self.folder, "plugin-stderr.log")
        stamp = time.strftime("%Y%m%d-%H%M%S")
        self.trace_path = None if args.no_trace else (args.trace or os.path.join(self.folder, f"trace-{stamp}.jsonl"))
        self.report_path = args.report or os.path.join(self.folder, f"report-{stamp}.json")
        self.trace = None
        self.plugin = None
        self._tokens_cache = (None, None)
        self.unknown = None  # a made-up name GET /users said nobody has
        self.verified = set()  # fingerprints of tokens Twitch confirmed belong to T
        self.wrong_account = None
        # Why the last sign-in was signed out at once (another account, or no approval in
        # time): later cases don't sign in again on their own, each waiting for minutes.
        self.sign_in_failed = None
        # V and R checked with Twitch and confirmed by the person: None (not yet), True, or
        # why every risky case is refused.
        self.accounts_checked = None
        self.pending_undo = {}  # what the running case changed: key -> (action, settings) that undoes it
        self.answers, self.saved = {}, {}
        self.records = []
        self.case_steps = []
        self.case_mark = self.prev_mark = self.cur_mark = 0
        self.case_trace = self.prev_trace = self.cur_trace = 0
        self.confirmed = READ
        self.started = time.time()
        self.interrupted = False

    # output and input
    def say(self, text=""):
        self.out.write(self.redactor.text(str(text)) + "\n")
        self.out.flush()

    def ask(self, prompt):
        try:
            return self.input_fn(self.redactor.text(prompt))
        except EOFError:
            return ""

    def on_log(self, level, message):
        if message.startswith("to sign in"):
            self.learn_user_code(message)
        if not self.args.quiet and (level != "info" or message.startswith(("signed", "to sign in", "Twitch sign-in"))):
            self.say(f"      plugin {level}: {message}")

    def learn_user_code(self, text):
        """Remember a device code from Sign in's answer or log line, so the report leaves
        it out (the console still shows it: the person types it in)."""
        m = USER_CODE.search(text or "")
        if m:
            self.codes.learn(m.group(1))

    def scrub(self, obj):
        """obj as it may be written to the report: no tokens, no device codes."""
        return self.codes.value(self.redactor.scrub(obj))

    # placeholders
    def placeholder(self, key):
        if key == "T":
            return self.T
        if key == "V":
            if not self.V:
                raise SkipCase("needs the test viewer (--viewer)")
            return self.V
        if key == "R":
            if not self.R:
                raise SkipCase("needs the raid target (--raid-target)")
            return self.R
        if key == "P":
            return self.P
        if key == "U":
            return self.ensure_unknown()
        if key == "STAMP":
            return self.stamp
        return "{" + key + "}"

    def shown(self, text):
        """Placeholders filled in where already known, for labels (no Twitch call)."""
        if not isinstance(text, str):
            return text
        known = {"T": self.T, "V": self.V, "R": self.R, "P": self.P, "U": self.unknown, "STAMP": self.stamp}
        return re.sub(r"\{(T|V|R|P|U|STAMP)\}", lambda m: known.get(m.group(1)) or m.group(0), text)

    def fmt(self, text, regex=False):
        if not isinstance(text, str):
            return text
        return re.sub(r"\{(T|V|R|P|U|STAMP)\}",
                      lambda m: re.escape(self.placeholder(m.group(1))) if regex else self.placeholder(m.group(1)),
                      text)

    # tokens and Twitch
    def tokens(self, learn=True):
        """tokens.json, read again only when it changed. On Windows an open file can't be
        replaced: since 1.1.3 the plugin's save waits for a reader (about 2 s at most), so
        the harness keeps its own reads few."""
        try:
            st = os.stat(self.tokens_path)
            signature = (st.st_mtime_ns, st.st_size)
        except OSError:
            signature = None
        if signature is None:
            tokens = None
        elif signature == self._tokens_cache[0]:
            tokens = self._tokens_cache[1]
        else:
            tokens = read_tokens(self.tokens_path)
            if tokens is not None:
                self._tokens_cache = (signature, tokens)
        if learn and tokens:
            self.redactor.learn_tokens(tokens)
        return dict(tokens) if tokens else None

    def signed_in_login(self):
        tokens = self.tokens()
        if not tokens or tokens.get("client_id") != self.client_id:
            return None
        return login_key(tokens.get("login")) or None

    def own_helix(self, method, path, params=None, with_headers=False):
        """One Helix request by the harness itself, with the plugin's token. Asks the
        plugin to refresh a token about to expire, never refreshes it itself (refresh
        tokens are single use)."""
        for attempt in (0, 1):
            tokens = self.tokens() or {}
            if not tokens.get("access_token"):
                raise SkipCase("not signed in")
            try:
                status, answer, headers = helix(method, path, params, token=tokens["access_token"],
                                                client_id=self.client_id, base=self.helix_url)
            except HelixError as e:
                raise SkipCase(str(e)) from None
            if status == 401 and attempt == 0 and float(tokens.get("expires_at") or 0) - time.time() < 300:
                self.nudge_refresh(tokens)
                continue
            break
        return (status, answer, headers) if with_headers else (status, answer)

    def nudge_refresh(self, tokens):
        old = token_key(tokens.get("access_token"))
        watching = list(self.plugin.watching)
        self.watch(watching + ["live:me"])
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and token_key((self.tokens() or {}).get("access_token")) == old:
            time.sleep(0.5)
        self.watch(watching)

    def verify_broadcaster(self):
        """Refuse unless the signed-in account is the test broadcaster: tokens.json says
        so, and Twitch agrees (GET /users with the token, once per token)."""
        login = self.signed_in_login()
        if login != self.T:
            raise Refusal(f"signed in as {login or 'nobody'}, not the test broadcaster {self.T}")
        key = token_key((self.tokens() or {}).get("access_token"))
        if key in self.verified:
            return
        try:
            status, answer = self.own_helix("GET", "/users")
        except SkipCase as e:
            raise Refusal(f"couldn't confirm the signed-in account with Twitch: {e}") from None
        if status != 200:
            raise Refusal(f"couldn't confirm the signed-in account with Twitch ({status}: {message_of(answer)})")
        actual = login_key(((answer.get("data") or [{}])[0]).get("login"))
        if actual != self.T:
            raise Refusal(f"Twitch says the token belongs to {actual or 'nobody'}, not the test broadcaster {self.T}")
        self.verified.add(key)

    def sign_out_now(self):
        """Press Sign out at once: the plugin cancels a sign-in still waiting for approval
        and revokes the saved token. Returns what happened, for the report."""
        self.verified.clear()
        try:
            a = self.plugin.run("sign-out", {})
        except Exception as e:  # report it with the sign-in's outcome
            return f"signing out failed ({type(e).__name__}: {e}); delete {self.folder} by hand"
        gone = not os.path.exists(self.tokens_path)
        if a.ok and not a.message and gone:
            return "signed out at once (any attempt still waiting cancelled, the token revoked)"
        said = "no answer in time" if a.timed_out else (a.message or "ok")
        return (f"signed out at once: {said}; tokens.json {'gone' if gone else 'still there'}"
                + ("" if gone else f" (delete {self.folder} by hand)"))

    def check_test_accounts(self):
        """Before the first case with a public effect: --viewer and --raid-target exist on
        Twitch, V follows T (the plan's 2.1), and the person confirms with a typed yes that
        these are the test accounts, so a typo can't point moderation or a raid at a
        stranger. Raises Refusal otherwise. Asked once a run; a "no" refuses every risky
        case after it."""
        if self.accounts_checked is True:
            return
        if self.accounts_checked:
            raise Refusal(self.accounts_checked)
        given = [(flag, name) for flag, name in (("--viewer", self.V), ("--raid-target", self.R)) if name]
        if not given:
            self.accounts_checked = True
            return

        def refuse(reason):  # an answer that won't change on its own: refuse every risky case
            self.accounts_checked = reason
            raise Refusal(reason)

        try:
            status, answer = self.own_helix("GET", "/users", [("login", n) for n in [self.T] + [n for _, n in given]])
        except SkipCase as e:
            raise Refusal(f"couldn't look the test accounts up on Twitch: {e}") from None
        if status != 200:
            raise Refusal(f"couldn't look the test accounts up (Twitch said {status}: {message_of(answer)})")
        found = {login_key(u.get("login")): u for u in answer.get("data") or []}
        missing = [f"{flag} {name}" for flag, name in given if name not in found]
        if missing:
            refuse(f"Twitch has no account called {', '.join(missing)}; check the spelling")
        follows = None
        if self.V:
            t_id = (found.get(self.T) or {}).get("id") or (self.tokens() or {}).get("user_id")
            try:
                status, answer = self.own_helix("GET", "/channels/followers",
                                                {"broadcaster_id": t_id, "user_id": found[self.V].get("id")})
            except SkipCase as e:
                raise Refusal(f"couldn't check that {self.V} follows {self.T}: {e}") from None
            if status != 200:
                raise Refusal(f"couldn't check that {self.V} follows {self.T} (Twitch said {status}: "
                              f"{message_of(answer)})")
            v_id = str(found[self.V].get("id"))
            follows = next((f for f in answer.get("data") or [] if str(f.get("user_id")) == v_id), None)
            if follows is None:
                refuse(f"the test viewer {self.V} doesn't follow {self.T} (the plan's 2.1: V follows T); "
                       "check --viewer, or follow T as V first")
        self.say("  The test accounts, as Twitch knows them:")
        for flag, name in given:
            u = found[name]
            role = ("the test viewer: banned, warned, made VIP and moderator in T's channel" if flag == "--viewer"
                    else "the raid target: raided, shouted out, chatted to and clipped")
            followed = f", follows {self.T} since {str(follows.get('followed_at') or '?')[:10]}" \
                if flag == "--viewer" and follows else ""
            self.say(f"    {flag} {u.get('login')}: id {u.get('id')}, created {str(u.get('created_at') or '?')[:10]}"
                     f"{followed}; {role}")
        if self.auto_yes:
            self.say("  [dry run against the fakes: answering yes]")
        elif self.ask("  Type yes if these are your own test accounts (anything else refuses every case with a "
                      "public effect): ").strip().lower() != "yes":
            refuse("the test accounts weren't confirmed (a typed yes); check --viewer and --raid-target")
        self.accounts_checked = True

    def ensure_unknown(self):
        """A made-up login GET /users says no account has (B1, G4)."""
        if self.unknown:
            return self.unknown
        for _ in range(5):
            name = login_key(self.make_login())
            if not re.fullmatch(r"[a-z0-9_]{4,25}", name):
                continue
            status, answer = self.own_helix("GET", "/users", {"login": name})
            if status == 200 and not answer.get("data"):
                self.unknown = name
                self.say(f"  made-up name {name}: GET /users says no account has it")
                return name
            if status == 200:
                self.say(f"  made-up name {name} belongs to an account; trying another")
                continue
            raise SkipCase(f"couldn't check a made-up name with GET /users (Twitch said {status}: {message_of(answer)})")
        raise SkipCase("every made-up name belonged to an account, or wasn't valid")

    def main_channel_problem(self, action):
        """Why this action mustn't happen in this run at all, sent by the harness or pressed
        by hand: the broadcaster is the main channel. None otherwise."""
        M = self.M
        if M and action in NEVER_ON_MAIN and M in (self.T, self.signed_in_login()):
            return (f"{action}: the broadcaster is the main channel ({M}), where the plan never runs moderation, "
                    "chat modes, Shield Mode, AutoMod, clearing chat, warnings or raids")
        return None

    def gate(self, action, settings):
        """Why the harness won't send this action, or None. Only for actions with a
        public effect: where it acts and on whom."""
        effect = ACTION_EFFECT.get(action, MODERATION)
        if RANK[effect] < RANK[PUBLIC]:
            return None
        T, V, R = self.T, self.V, self.R
        problem = self.main_channel_problem(action)
        if problem:
            return problem
        channel = login_key(settings.get("channel"))
        if channel in ("", "me"):
            channel = T
        if action in MOD_CHANNEL_ACTIONS and channel != T:
            return f"{action} in {channel}'s channel: the harness moderates only the test broadcaster's channel ({T})"
        if action in POST_ACTIONS and channel not in {T, R} - {None}:
            return f"{action} in {channel}'s channel: the harness posts only in {T}'s or the raid target's channel"
        if action == "raid" and (R is None or channel != R):
            return f"raid {channel}: the harness raids only the raid target (--raid-target)"
        if action == "shoutout" and channel not in {T, R} - {None}:
            return f"shoutout {channel}: the harness shouts out only the raid target (or {T} itself)"
        if action in USER_ACTIONS:
            target = login_key(settings.get("user"))
            allowed = {V, self.unknown} | ({T} if action == "ban" else set())
            allowed.discard(None)
            if not target or target not in allowed:
                return (f"{action} {target or '(nobody)'}: the harness moderates only the test viewer "
                        f"({V or 'not given: --viewer'}), or a made-up name GET /users confirmed nobody has")
        if effect == MODERATION and action not in MOD_CHANNEL_ACTIONS | USER_ACTIONS:
            return f"{action}: the harness doesn't know where this acts, so it won't send it"
        return None

    def allow(self, action, settings):
        """Called before every action: refuses (raises Refusal) what the case wasn't
        confirmed for, what the gate refuses, and anything risky while signed in as
        someone other than the test broadcaster."""
        effect = ACTION_EFFECT.get(action, MODERATION)
        if RANK[effect] > RANK[self.confirmed]:
            raise Refusal(f"{action} has a {effect} effect, which this case wasn't confirmed for")
        if RANK[effect] >= RANK[PUBLIC]:
            problem = self.gate(action, settings)
            if problem:
                raise Refusal(problem)
            self.verify_broadcaster()

    @staticmethod
    def public_actions(case):
        """The actions with a public effect a case runs: its Run steps, and the actions its
        Custom steps declare."""
        runs = []
        for step in case.steps:
            for run in ([step] if isinstance(step, Run) else getattr(step, "actions", ())):
                if RANK[run.effect] >= RANK[PUBLIC]:
                    runs.append(run)
        return runs

    @staticmethod
    def acts_publicly(case):
        """The harness itself does something public in this case (not only a person)."""
        return any(RANK[step.effect] >= RANK[PUBLIC] for step in case.steps)

    def where(self, run):
        """Where an action acts, for the yes prompt."""
        target = run.target(self)
        channel = login_key(target.get("channel"))
        if channel in ("", "me"):
            channel = self.T
        if run.action == "raid":
            return f"from {self.T}'s channel to {channel}'s"
        if run.action == "shoutout":
            return f"in {self.T}'s chat, for {channel}"
        if target.get("user"):
            return f"on {login_key(target['user'])}, in {channel}'s channel"
        return f"in {channel}'s channel"

    def confirm(self, case):
        """Stop for a typed yes before a case with a public effect, listing each action
        and where it acts."""
        self.say(f"  {case.id} has a {case.effect} effect. It runs:")
        for run in self.public_actions(case):
            self.say(f"    {run.describe(self)}, {self.where(run)}")
        for step in case.steps:
            if getattr(step, "by_hand", ()):
                self.say(f"    and by hand: {', '.join(step.by_hand)}")
        if case.confirm_note:
            self.say(f"    ({case.confirm_note})")
        if self.auto_yes:
            self.say("  [dry run against the fakes: answering yes]")
            return True
        answer = self.ask(f"  Type yes to run {case.id} (anything else skips it): ")
        return answer.strip().lower() == "yes"

    def preflight(self, case):
        for step in case.steps:
            if RANK[step.effect] > RANK[case.effect]:
                raise Refusal(f"{case.id} is declared {case.effect} but has a {step.effect} step: {step.describe(self)}")
        for run in self.public_actions(case):
            problem = self.gate(run.action, run.target(self))
            if problem:
                raise Refusal(problem)
        for step in case.steps:
            for action in getattr(step, "by_hand", ()):
                problem = self.main_channel_problem(action)
                if problem:
                    raise Refusal("by hand: " + problem)

    def ensure_signed_in(self, case):
        login = self.signed_in_login()
        if login == self.T:
            return
        if login:
            why = f"signed in as {login}, not the test broadcaster {self.T}"
            if case.risky:
                raise Refusal(why)
            raise SkipCase(why)
        if self.sign_in_failed:
            why = f"the sign-in earlier in this run didn't work ({self.sign_in_failed}); run the harness again"
            if case.risky:
                raise Refusal(why)
            raise SkipCase(why)
        if self.person or self.fake is not None:
            status, detail, _ = self.do(SignIn())
            if self.signed_in_login() == self.T:
                return
            if case.risky:
                raise Refusal(detail)
            raise SkipCase(detail)
        raise SkipCase("not signed in as the test broadcaster; sign in first (run without --auto-only)")

    # the plugin
    def command(self):
        if self.trace_path is None and self.fake is None:
            return [sys.executable, PLUGIN_PY]
        command = [sys.executable, TRACE_PY, "run"]
        if self.trace_path:
            command += ["--trace", self.trace_path]
        if self.fake is not None:
            import fake_twitch
            command += fake_twitch.plugin_overrides(self.fake)
        return command

    def start_plugin(self):
        self.plugin.start(self.settings)

    def await_start_check(self, timeout=15):
        """A signed-in plugin checks its token once after it starts (START_LINE says how it
        went), and may refresh it then. Waits for that, so a step comparing tokens before
        and after doesn't see the refresh. Going on without it is safe: the line can't be
        taken for a sign-in's outcome, whenever it comes."""
        if not self.tokens():
            return
        start = self.plugin.started_at
        self.plugin.wait(lambda: any(m.startswith(START_LINE) for _, _, m in self.plugin.logs_since(start)), timeout)

    def restart_plugin(self, kill=False):
        self.plugin.stop(kill=kill)
        self.start_plugin()

    def send_settings(self):
        self.plugin.send({"event": "settings", "settings": self.settings})

    def watch(self, values):
        self.plugin.watch(values)

    def wait_value(self, vid, pattern, timeout, after=None):
        rx = re.compile(pattern)

        def ok():
            if after is not None:
                events = [v for s, i, v in self.plugin.value_events if i == vid and s > after]
                return bool(events) and bool(rx.fullmatch(events[-1]))
            value = self.plugin.values.get(vid)
            return value is not None and bool(rx.fullmatch(value))

        return self.plugin.wait(ok, timeout)

    def plugin_state(self):
        try:
            with open(os.path.join(self.data_dir, "state.json"), encoding="utf-8") as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    # running steps and cases
    def do(self, step):
        if step.optional and not (self.person and self.args.check_by_eye):
            status, detail, extra = SKIPPED, f"check by eye (--check-by-eye): {step.describe(self)}", {}
            seconds = 0.0
        else:
            self.prev_mark, self.cur_mark = self.cur_mark, self.plugin.mark()
            self.prev_trace, self.cur_trace = self.cur_trace, (self.trace.last_seq() if self.trace else 0)
            start = time.monotonic()
            out = step.run(self)
            seconds = time.monotonic() - start
            status, detail = out[0], out[1]
            extra = out[2] if len(out) > 2 else {}
        record = {"step": extra.pop("step", None) or step.describe(self), "status": status, "detail": detail,
                  "seconds": round(seconds, 1), **extra}
        record = self.scrub(record)
        self.case_steps.append(record)
        if status != DONE or self.args.verbose:
            self.say(f"    {status:8s} {record['step']}: {record['detail']}")
        return status, detail, extra

    def note_undo(self, action, settings, answer):
        """After an action: remember how to put back what it changed, or forget that once
        it's put back, in case the case stops early."""
        found = undo_for(action, settings, self.answers)
        if not found:
            return
        key, undo = found
        if undo is None:
            if answer.ok:
                self.pending_undo.pop(key, None)
        elif answer.ok or answer.timed_out:  # no answer in time: it may still have happened
            self.pending_undo.pop(key, None)
            self.pending_undo[key] = undo  # the newest last, undone first

    def run_undo(self, case):
        """A case stopped early: put back what it changed, newest first, each action
        through the same checks as any other (allow())."""
        pending = list(self.pending_undo.values())[::-1]
        self.pending_undo = {}
        self.say(f"   {case.id} stopped early; putting back what it changed:")
        for action, settings in pending:
            label = f"undo: run {action}" + (" (" + ", ".join(f"{k}={v!r}" for k, v in settings.items()) + ")"
                                             if settings else "")
            try:
                if self.plugin.exited:
                    self.start_plugin()
                self.allow(action, settings)
                a = self.plugin.run(action, settings)
                status = DONE if a.ok else UNDO_FAILED
                detail = ("ok" + (f": {a.message}" if a.message else "")) if a.ok else \
                    (a.message or f"no answer within {ACTION_TIMEOUT:g} s")
            except Refusal as e:
                status, detail = UNDO_FAILED, f"refused: {e}"
            except KeyboardInterrupt:
                status, detail = UNDO_FAILED, "interrupted; the rest isn't undone either"
            except Exception as e:  # report it and go on with the rest
                status, detail = UNDO_FAILED, f"{type(e).__name__}: {e}"
            record = self.scrub({"step": label, "status": status, "detail": detail, "seconds": 0.0})
            self.case_steps.append(record)
            self.say(f"    {'undone' if status == DONE else 'NOT UNDONE':10s} {record['step']}: {record['detail']}")
            if detail.startswith("interrupted"):
                break

    def only_ids(self):
        """The cases --only names one by one (not by section)."""
        return {x.strip().upper() for x in (self.args.only or "").split(",") if CASE_ID.match(x.strip().upper())}

    def why_not(self, case):
        """Why a selected case doesn't run, as (status, reason), or None."""
        if self.args.skip_affiliate and case.section in AFFILIATE_SECTIONS:
            return NEEDS_AFFILIATE, "needs an Affiliate channel (--no-skip-affiliate runs it on an Affiliate's channel)"
        if (not self.args.skip_affiliate and case.risky and case.section not in AFFILIATE_SECTIONS
                and case.id not in self.only_ids()):
            return REFUSED, (f"--no-skip-affiliate runs on an Affiliate's channel, often the main one: a case with a "
                             f"public effect outside I, J and K runs only when --only names it (--only {case.id})")
        if self.fake is not None and not case.fake:
            return SKIPPED, "not part of the dry run against the fakes"
        if not self.person and self.fake is None and case.how != AUTO:
            return MANUAL_RESULT, "needs a person (run without --auto-only)"
        if not self.person and self.fake is None and case.risky:
            return SKIPPED, "has a public effect, which needs a typed yes (run without --auto-only)"
        for need, ok, reason in (("viewer", self.V, "needs the test viewer (--viewer)"),
                                 ("raid_target", self.R, "needs the raid target (--raid-target)"),
                                 ("confidential", self.args.confidential_client_id,
                                  "needs a Confidential app's client ID (--confidential-client-id)"),
                                 ("windows", sys.platform == "win32", "Windows only"),
                                 ("posix", os.name == "posix", "macOS and Linux only")):
            if need in case.needs and not ok:
                return SKIPPED, reason
        return None

    def run_case(self, case):
        self.say("")
        self.say(f"== {case.id} (stage {case.stage}, {case.how}, {case.effect}): {self.shown(case.title)}")
        rec = {"id": case.id, "stage": case.stage, "title": case.title, "how": case.how, "effect": case.effect}
        why = self.why_not(case)
        if why:
            rec.update(status=why[0], reason=why[1], steps=[])
            self.say(f"   {why[0]}: {why[1]}")
            return rec
        self.case_steps = []
        self.case_mark = self.prev_mark = self.cur_mark = self.plugin.mark()
        self.case_trace = self.prev_trace = self.cur_trace = self.trace.last_seq() if self.trace else 0
        self.confirmed = PRIVATE  # what setting up the case (signing in) needs
        self.pending_undo = {}
        start = time.monotonic()
        status, reason = None, None
        try:
            if case.signed_in:
                self.ensure_signed_in(case)
            self.preflight(case)
            if self.acts_publicly(case):
                self.verify_broadcaster()
                self.check_test_accounts()
                if not self.confirm(case):
                    raise SkipCase("not confirmed (type yes to run it)")
            elif case.risky:  # a person does the public part (F2, G9), in V's or R's name
                self.verify_broadcaster()
                self.check_test_accounts()
            # A risky case whose public part a person does (F2, G9) isn't confirmed for more
            # than its set-up.
            self.confirmed = case.effect if self.acts_publicly(case) or not case.risky else PRIVATE
            for step in case.steps:
                self.do(step)
        except SkipCase as e:
            status, reason = e.status, str(e)
        except Refusal as e:
            status, reason = REFUSED, str(e)
            self.say(f"    refused  {e}")
        except KeyboardInterrupt:
            self.interrupted = True
            status, reason = SKIPPED, "interrupted"
        except Exception as e:  # a harness or plugin problem: report it and carry on
            sys.stderr.write(self.redactor.text(traceback.format_exc()))
            sys.stderr.flush()
            status, reason = FAIL, f"harness error: {type(e).__name__}: {e}"
        finally:
            try:
                if status is not None and self.pending_undo:
                    self.run_undo(case)
            finally:
                self.confirmed = READ
                self.tidy_up(case)
        judged = [s["status"] for s in self.case_steps if s["status"] not in (DONE, UNDO_FAILED)]
        final = (FAIL if FAIL in judged or status == FAIL else REFUSED if status == REFUSED
                 else PASS if PASS in judged else RECORDED if RECORDED in judged else status or SKIPPED)
        rec.update(status=final, seconds=round(time.monotonic() - start, 1), steps=self.case_steps)
        if reason and final in (PASS, RECORDED):
            rec["incomplete"] = reason
        elif reason:
            rec["reason"] = reason
        left = [s["step"][len("undo: "):] for s in self.case_steps if s["status"] == UNDO_FAILED]
        if left:
            rec["not_undone"] = left
            self.say(f"   {case.id} left these as they were; put them back by hand: {'; '.join(left)}")
        self.say(f"   {case.id}: {final}" + (f" ({reason})" if reason else ""))
        return self.scrub(rec)

    def tidy_up(self, case):
        try:
            if self.plugin.exited:
                self.start_plugin()
            if self.settings != self.baseline:
                self.settings = dict(self.baseline)
                self.send_settings()
            if self.plugin.watching:
                self.watch([])
        except Exception as e:
            self.say(f"   couldn't tidy up after {case.id}: {e}")

    def selected(self, all_cases):
        only = [x.strip().upper() for x in (self.args.only or "").split(",") if x.strip()]
        if not only:
            return list(all_cases)
        return [c for c in all_cases if c.id in only or c.section in only]

    def summary(self):
        counts = {}
        for r in self.records:
            counts[r["status"]] = counts.get(r["status"], 0) + 1
        return counts

    def write_report(self, finished=False):
        report = {
            "tool": "plugins/tools/twitch_live_test.py",
            "mode": "dry run against the fakes" if self.fake is not None else "live",
            "plugin_version": manifest().get("version"),
            "started": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(self.started)),
            "finished": time.strftime("%Y-%m-%dT%H:%M:%S") if finished else None,
            "interrupted": self.interrupted,
            "accounts": {"broadcaster": self.T, "viewer": self.V, "raid_target": self.R, "public_channel": self.P,
                         "main_channel": self.M},
            "accounts_confirmed": self.accounts_checked is True,
            "signed_in_as": self.signed_in_login(),
            "wrong_account_signed_out": self.wrong_account,
            "made_up_name": self.unknown,
            "options": {"only": self.args.only, "skip_affiliate": self.args.skip_affiliate,
                        "auto_only": self.args.auto_only, "check_by_eye": self.args.check_by_eye,
                        "trace": self.trace_path},
            "summary": self.summary(),
            "cases": self.records,
        }
        report = self.scrub(report)
        folder = os.path.dirname(os.path.abspath(self.report_path))
        os.makedirs(folder, exist_ok=True)
        tmp = self.report_path + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(report, f, indent=2, ensure_ascii=False)
            f.write("\n")
        os.replace(tmp, self.report_path)

    def prepare(self):
        prepare_test_folder(self.folder)
        aside = self.tokens_path + ".harness-aside"
        if os.path.exists(aside) and not os.path.exists(self.tokens_path):
            os.replace(aside, self.tokens_path)  # a run stopped during B9
        if self.trace_path:
            self.trace = twitch_trace.TraceReader(self.trace_path)
        self.plugin = PluginProcess(self.command(), self.data_dir, self.stderr_path, self.on_log)
        self.start_plugin()
        self.say(f"Test folder: {self.folder}")
        if self.trace_path:
            self.say(f"Trace: {self.trace_path}")
        self.say(f"Report: {self.report_path}")
        if self.signed_in_login():
            self.say(f"Signed in already as {self.signed_in_login()}")

    def run(self, all_cases=None):
        all_cases = all_cases if all_cases is not None else cases()
        problems = check_definitions(all_cases)
        if problems:
            raise SystemExit("case definitions: " + "; ".join(problems))
        self.prepare()
        try:
            for case in self.selected(all_cases):
                self.records.append(self.run_case(case))
                self.write_report()
                if self.interrupted:
                    break
        except KeyboardInterrupt:
            self.interrupted = True
        finally:
            try:
                if self.args.sign_out_at_end and self.signed_in_login():
                    a = self.plugin.run("sign-out", {})
                    self.say(f"Signed out: {'ok' if a.ok else a.message}")
            finally:
                self.plugin.stop()
                self.write_report(finished=True)
        counts = self.summary()
        self.say("")
        self.say("Summary: " + (", ".join(f"{n} {s}" for s, n in sorted(counts.items())) or "nothing ran"))
        self.say(f"Report: {self.report_path}")
        if self.fake is None and self.signed_in_login():
            self.say(f"The sign-in is kept in {self.folder} for the next run; --sign-out-at-end ends it at Twitch, "
                     "and the folder can be deleted when you're done.")
        return 1 if counts.get(FAIL) else (130 if self.interrupted else 0)


def parse_args(argv=None):
    p = argparse.ArgumentParser(
        description="Run the Twitch plugin's real-world test plan (docs/testing/twitch.md) through its stdio "
                    "protocol and write a JSON report. Cases with a public effect wait for a typed yes.")
    p.add_argument("--broadcaster", help="T, the test broadcaster's login (the account that signs in)")
    p.add_argument("--viewer", help="V, the test viewer's login: the only account moderation targets")
    p.add_argument("--raid-target", help="R, the raid target's login: raids, shoutouts, chat in R's channel")
    p.add_argument("--main-channel", help="M, the main channel's login (the maintainer's real channel): whenever "
                   "the broadcaster is M, moderation, chat modes, Shield Mode, AutoMod, clearing chat, warnings "
                   "and raids are refused. Always worth giving")
    p.add_argument("--public-channel", default="twitch",
                   help="a large public channel for the read-only cases (default: twitch)")
    p.add_argument("--b8-channels", default="twitchdev,twitchgaming,twitchmusic,twitchrivals,twitchpresents,"
                   "twitchsupport", help="more public channels for B8 (only those GET /users knows are used)")
    p.add_argument("--client-id", help=f"your Twitch app's client ID (or set {CLIENT_ID_ENV}; asked for otherwise)")
    p.add_argument("--confidential-client-id", help="a Confidential app's client ID, for A6")
    p.add_argument("--only", help="sections or cases to run, e.g. A,B or A3,B1 (default: all, in the plan's order)")
    p.add_argument("--skip-affiliate", dest="skip_affiliate", action="store_true", default=True,
                   help="report sections I, J and K as needing an Affiliate channel (the default)")
    p.add_argument("--no-skip-affiliate", dest="skip_affiliate", action="store_false",
                   help="run sections I, J and K too (the signed-in channel must be Affiliate or Partner); every "
                        "other case with a public effect is then refused unless --only names it, e.g. --only C1")
    p.add_argument("--auto-only", action="store_true",
                   help="unattended: only cases that need no person and have no public effect")
    p.add_argument("--check-by-eye", action="store_true", help="also ask the optional by-eye questions")
    p.add_argument("--fresh-sign-in", action="store_true", help="sign in again in A3 even if already signed in")
    p.add_argument("--sign-out-at-end", action="store_true", help="sign out (revoking the token) when done")
    p.add_argument("--sign-in-minutes", type=float, default=10, help="how long to wait for an approval (10)")
    p.add_argument("--soak-minutes", type=float, default=120, help="M1's soak (120; 0 skips it)")
    p.add_argument("--validate-minutes", type=float, default=75, help="each half of A8 (75; 0 skips it)")
    p.add_argument("--l10-minutes", type=float, default=10, help="how long L10 watches (10)")
    p.add_argument("--data-dir", default=DEFAULT_TEST_FOLDER,
                   help=f"the test folder (default {DEFAULT_TEST_FOLDER}); never PerritoDeck's real one")
    p.add_argument("--report", help="where to write the JSON report (default: in the test folder)")
    p.add_argument("--trace", help="where the HTTP trace goes (default: in the test folder)")
    p.add_argument("--no-trace", action="store_true", help="run plugin.py itself, without the HTTP trace")
    p.add_argument("--dry-run-fake", action="store_true",
                   help="run against the fake Twitch from plugins/twitch/test_plugin.py, answering yes by itself")
    p.add_argument("--keep-test-folder", action="store_true",
                   help="with --dry-run-fake: keep the test folder (plugin stderr, state) to look at")
    p.add_argument("--list", action="store_true", help="list the cases in order and exit")
    p.add_argument("--quiet", action="store_true", help="don't echo the plugin's warnings")
    p.add_argument("--verbose", action="store_true", help="also show set-up steps")
    return p.parse_args(argv)


def check_accounts(args):
    names = {"--broadcaster": args.broadcaster, "--viewer": args.viewer, "--raid-target": args.raid_target,
             "--public-channel": args.public_channel, "--main-channel": args.main_channel}
    for flag, name in names.items():
        if name and not LOGIN.match(login_key(name)):
            return f"{flag} {name!r} isn't a Twitch login (up to 25 letters, digits or _)"
    t, v, r, p, m = (login_key(x) if x else None for x in names.values())
    if v and v == t:
        return "the test viewer must be another account than the test broadcaster"
    if r and r in (t, v):
        return "the raid target must be a third account"
    if p and p == t:
        return "the public channel must be someone else's (a channel T doesn't moderate)"
    if m and m in (v, r):
        return "the main channel can't be the test viewer or the raid target: those are moderated and raided"
    return None


def main(argv=None, input_fn=input):
    args = parse_args(argv)
    try:
        sys.stdout.reconfigure(errors="replace")
    except (AttributeError, ValueError):
        pass
    if args.list:
        for c in cases():
            print(f"{c.stage:2d}  {c.id:4s} {c.how:6s} {c.effect:10s} {c.title}")
        return 0
    fake = None
    temp = None
    if args.dry_run_fake:
        import fake_twitch
        fake = fake_twitch.FakeTwitch()
        args.broadcaster = args.broadcaster or fake_twitch.ME["login"]
        args.viewer = args.viewer or fake_twitch.VIEWER["login"]
        args.raid_target = args.raid_target or fake_twitch.OTHER["login"]
        args.public_channel = fake_twitch.OTHER["login"] if args.public_channel == "twitch" else args.public_channel
        args.client_id = fake_twitch.CLIENT_ID
        temp = tempfile.mkdtemp(prefix="twitch-dry-run-")
        args.data_dir = os.path.join(temp, "test-folder")
        stamp = time.strftime("%Y%m%d-%H%M%S")
        out = os.path.join(tempfile.gettempdir(), f"perritodeck-twitch-dry-run-{stamp}")
        args.report = args.report or out + "-report.json"
        args.trace = args.trace or out + "-trace.jsonl"
    else:
        if not args.broadcaster:
            print("--broadcaster is required: the test broadcaster's login (see docs/testing/twitch.md, 2.1)",
                  file=sys.stderr)
            return 2
        if not args.skip_affiliate and not args.main_channel:
            print("--no-skip-affiliate needs --main-channel: it usually runs on the main channel, and --main-channel "
                  "is what keeps moderation and raids off it", file=sys.stderr)
            return 2
        args.client_id = (args.client_id or os.environ.get(CLIENT_ID_ENV) or "").strip()
        if not args.client_id:
            args.client_id = input_fn("Client ID of your Twitch app (client type Public): ").strip()
        if not args.client_id:
            print("no client ID", file=sys.stderr)
            return 2
        problem = folder_problem(args.data_dir)
        if problem:
            print(problem, file=sys.stderr)
            return 2
    problem = check_accounts(args) or next(
        (location_problem(path) for path in (args.trace, args.report) if path and location_problem(path)), None)
    if problem:
        print(problem, file=sys.stderr)
        if temp:
            shutil.rmtree(temp, ignore_errors=True)
        if fake is not None:
            fake.stop()
        return 2
    try:
        return Harness(args, fake=fake, input_fn=input_fn).run()
    finally:
        if fake is not None:
            fake.stop()
        if temp and args.keep_test_folder:
            print(f"Dry run: the trace is {args.trace}; the test folder is {args.data_dir}")
        elif temp:
            shutil.rmtree(temp, ignore_errors=True)
            print(f"Dry run: the trace is {args.trace}; the test folder (with the fake sign-in) was deleted.")


if __name__ == "__main__":
    sys.exit(main())
