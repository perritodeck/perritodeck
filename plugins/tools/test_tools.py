"""Tests of the Twitch test tools in this folder, run against the fake Twitch from
plugins/twitch/test_plugin.py. The harness's safety rules come first: it stops for a typed
yes before a public effect, refuses risky cases unless the signed-in account is the test
broadcaster, moderates only the test viewer in the test broadcaster's channel, and checks
a made-up name with GET /users before using it.

    py -3 -m unittest test_tools        (from plugins/tools; plugins/test_all.py runs it)
"""

import contextlib
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import types
import unittest
import zipfile
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import fake_twitch  # noqa: E402
import twitch_cleanup  # noqa: E402
import twitch_common  # noqa: E402
import twitch_live_test as live  # noqa: E402
import twitch_trace  # noqa: E402

T, V, R = fake_twitch.ME["login"], fake_twitch.VIEWER["login"], fake_twitch.OTHER["login"]
SECRETS = ("access-0", "access-1", "access-r", "refresh-0", "refresh-1", "refresh-r", "dev-code-1")
CHANGES = ("POST", "PUT", "PATCH", "DELETE")


class WithFake(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.fake = fake_twitch.FakeTwitch()

    @classmethod
    def tearDownClass(cls):
        cls.fake.stop()

    def setUp(self):
        self.fake.reset()
        self.tmp = tempfile.mkdtemp(prefix="twitch-tools-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.prompts = []

    def args(self, *extra, broadcaster=T, viewer=V, raid_target=R, public=R):
        args = live.parse_args(["--broadcaster", broadcaster, "--viewer", viewer, "--raid-target", raid_target,
                                "--public-channel", public, "--data-dir", os.path.join(self.tmp, "test-folder"),
                                "--report", os.path.join(self.tmp, "report.json"),
                                "--trace", os.path.join(self.tmp, "trace.jsonl"), *extra])
        args.client_id = fake_twitch.CLIENT_ID
        return args

    def harness(self, *extra, answers=(), auto_yes=False, make_login=live.made_up_login, **kw):
        answers = list(answers)

        def answer(prompt):
            self.prompts.append(prompt)
            return answers.pop(0) if answers else ""

        self.out = io.StringIO()
        return live.Harness(self.args(*extra, **kw), fake=self.fake, input_fn=answer, out=self.out,
                            auto_yes=auto_yes, make_login=make_login)

    def run_harness(self, *extra, cases=None, **kw):
        h = self.harness(*extra, **kw)
        h.run(cases)
        with open(h.report_path, encoding="utf-8") as f:
            report = json.load(f)
        return {c["id"]: c for c in report["cases"]}, report

    def changes(self):
        """Requests to the fake Helix that change something (EventSub subscriptions aside)."""
        return [r for r in self.fake.requests() if r[0] in CHANGES and r[1] != "/eventsub/subscriptions"]

    def sign_in(self):
        cases, _ = self.run_harness("--only", "A3")
        self.assertEqual(cases["A3"]["status"], live.PASS, cases["A3"])

    def tokens_path(self):
        return os.path.join(twitch_common.plugin_data(os.path.join(self.tmp, "test-folder")), "tokens.json")


ACCOUNTS_PROMPT = "Type yes if these are your own test accounts"


class SafetyTest(WithFake):
    def test_public_effect_cases_stop_for_a_typed_yes(self):
        self.sign_in()
        # Read-only: runs without asking.
        cases, _ = self.run_harness("--only", "B3")
        self.assertEqual(cases["B3"]["status"], live.PASS)
        self.assertEqual(self.prompts, [])
        # Public: anything but a typed yes skips it, and nothing is sent. (The first yes
        # confirms the test accounts.)
        for answer in ("no", "y", "", "yes please"):
            cases, _ = self.run_harness("--only", "C7", answers=["yes", answer])
            self.assertEqual(cases["C7"]["status"], live.SKIPPED, answer)
            self.assertIn("not confirmed", cases["C7"]["reason"])
            self.assertTrue(self.prompts[-1].strip().startswith("Type yes to run C7"))
            self.assertEqual(self.changes(), [], answer)
        cases, _ = self.run_harness("--only", "C7", answers=["yes", "YES"])
        self.assertEqual(cases["C7"]["status"], live.PASS)
        self.assertEqual([r[:2] for r in self.changes()], [("PATCH", "/channels")] * 4)
        self.assertIn("C7 has a public effect. It runs:", self.out.getvalue())
        self.assertIn("run branded (state='toggle'), in perritodev's channel", self.out.getvalue())

    def test_the_yes_prompt_lists_every_action_and_where_it_acts(self):
        self.sign_in()
        self.run_harness("--only", "G7,E1,C1", auto_yes=True)
        out = self.out.getvalue()
        # The VIP toggles inside G7's custom step; chat in R's channel; C1's restore.
        self.assertEqual(out.count("run vip (user='someviewer', change='toggle'), on someviewer, in perritodev's "
                                   "channel"), 2)
        self.assertIn("run chat (message='PerritoDeck test: hello from the test harness', channel='otherstreamer'), "
                      "in otherstreamer's channel", out)
        self.assertIn("run chat (message='PerritoDeck test: hello from the test harness'), in perritodev's channel",
                      out)
        self.assertIn("run set-info (title=(set when it runs), category=(set when it runs)), in perritodev's channel",
                      out)
        self.assertIn("run tags (mode='set', tags=(set when it runs)), in perritodev's channel", out)
        h = self.harness()
        raid = live.Run("raid", channel="{R}")
        self.assertEqual(h.where(raid), "from perritodev's channel to otherstreamer's")
        self.assertEqual(h.where(live.Run("shoutout", channel="{R}")), "in perritodev's chat, for otherstreamer")
        # A custom step's declared actions are checked before asking, like plain steps.
        hidden = live.Case("X5", 9, "A ban of someone else, declared", live.AUTO, live.MODERATION, fake=True, steps=[
            live.Custom(lambda h: None, "ban", effect=live.MODERATION, actions=[live.Run("ban", user=R)])])
        with self.assertRaisesRegex(live.Refusal, "moderates only the test viewer"):
            h.preflight(hidden)

    def test_unattended_runs_skip_public_effects(self):
        self.sign_in()
        args = self.args("--only", "B3,C7,A7", "--auto-only")
        h = live.Harness(args, fake=None, out=io.StringIO())  # as a live run is; nothing started
        self.assertIsNone(h.why_not(next(c for c in live.cases() if c.id == "B3")))
        self.assertEqual(h.why_not(next(c for c in live.cases() if c.id == "C7"))[0], live.SKIPPED)
        self.assertEqual(h.why_not(next(c for c in live.cases() if c.id == "A7"))[0], live.MANUAL_RESULT)
        # And a live run never answers yes by itself.
        self.assertFalse(live.Harness(args, fake=None, auto_yes=True).auto_yes)

    def test_refused_unless_signed_in_as_the_test_broadcaster(self):
        # The fake signs in as perritodev; the harness was told the broadcaster is someone else.
        cases, _ = self.run_harness("--only", "A3,C1,G2,E4", broadcaster=R, raid_target="zx", public="zx",
                                    auto_yes=True)
        self.assertEqual(cases["A3"]["status"], live.FAIL)
        self.assertIn(f"signed in as {T}, not the test broadcaster {R}", json.dumps(cases["A3"]))
        for cid in ("C1", "G2"):
            self.assertEqual(cases[cid]["status"], live.REFUSED, cid)
            self.assertIn(f"signed in as {T}, not the test broadcaster {R}", cases[cid]["reason"])
        self.assertEqual(cases["E4"]["status"], live.SKIPPED)  # not risky, but not T's channel either
        self.assertEqual(self.changes(), [])
        # The wrong sign-in was ended at once, and the later cases didn't ask for another.
        self.assertEqual(self.fake.state["revoked"], ["access-1"])
        self.assertIn("the sign-in earlier in this run didn't work", cases["E4"]["reason"])

    def test_twitch_must_agree_on_the_account(self):
        self.sign_in()
        tokens_path = os.path.join(twitch_common.plugin_data(os.path.join(self.tmp, "test-folder")), "tokens.json")
        tokens = twitch_common.read_tokens(tokens_path)
        tokens["login"] = R  # tokens.json claims another account than the token's
        twitch_common.write_json_atomically(tokens_path, tokens)
        # (Checked without the plugin running: it would put the right login back when it
        # validates at start.)
        h = self.harness(broadcaster=R, raid_target="zx", public="zx")
        self.assertEqual(h.signed_in_login(), R)
        with self.assertRaisesRegex(live.Refusal, f"Twitch says the token belongs to {T}, not the test broadcaster"):
            h.verify_broadcaster()
        self.assertEqual(self.changes(), [])

    def test_moderation_only_on_the_test_viewer_in_the_test_channel(self):
        h = self.harness()
        allowed = [("ban", {"user": V}), ("ban", {"user": "@SomeViewer", "channel": "me"}),
                   ("ban", {"user": T}), ("unban", {"user": V, "channel": T}), ("warn", {"user": V}),
                   ("vip", {"user": V}), ("mod", {"user": V}), ("chat-mode", {"mode": "emote"}),
                   ("clear-chat", {}), ("shield", {"channel": T}), ("raid", {"channel": R}),
                   ("shoutout", {"channel": R}), ("shoutout", {"channel": T}), ("chat", {"channel": R}),
                   ("chat", {}), ("clip", {"channel": R}), ("chat-color", {"color": "blue"}),
                   ("set-info", {"title": "x"})]
        for action, settings in allowed:
            self.assertIsNone(h.gate(action, settings), (action, settings))
        refused = [("ban", {"user": R}), ("ban", {"user": V, "channel": R}), ("ban", {"user": ""}),
                   ("unban", {"user": T}), ("warn", {"user": "stranger"}), ("vip", {"user": R}),
                   ("mod", {"user": "zx"}), ("chat-mode", {"mode": "emote", "channel": R}),
                   ("clear-chat", {"channel": "zx"}), ("automod", {"level": "1", "channel": R}),
                   ("raid", {"channel": "zx"}), ("raid", {}), ("shoutout", {"channel": "zx"}),
                   ("chat", {"channel": "zx"}), ("announce", {"channel": "zx"}), ("nuke", {}),
                   ("ban", {"user": "pdmadeupname0001"})]
        for action, settings in refused:
            self.assertIsNotNone(h.gate(action, settings), (action, settings))
        h.unknown = "pdmadeupname0001"  # once GET /users has confirmed nobody has it
        self.assertIsNone(h.gate("ban", {"user": "pdmadeupname0001"}))
        self.assertIsNotNone(h.gate("warn", {"user": "zx"}))
        # Without a test viewer, no user is a target.
        h.V = None
        self.assertIsNotNone(h.gate("ban", {"user": V}))

    def test_a_refused_moderation_case_sends_nothing(self):
        self.sign_in()
        stranger = live.Case("X1", 9, "Ban someone else", live.AUTO, live.MODERATION, needs={"viewer"}, fake=True,
                             steps=[live.Run("ban", user=R, duration="60")])
        elsewhere = live.Case("X2", 9, "Ban V in R's channel", live.AUTO, live.MODERATION, needs={"viewer"},
                              fake=True, steps=[live.Run("ban", user="{V}", duration="60", channel="{R}")])
        hidden = live.Case("X3", 9, "A ban inside a custom step", live.AUTO, live.MODERATION, fake=True, steps=[
            live.Custom(lambda h: h.do(live.Run("ban", user=R, duration="60")) and None, "ban",
                        effect=live.MODERATION)])
        cases, _ = self.run_harness(cases=[stranger, elsewhere, hidden], answers=["yes", "yes"])
        self.assertEqual(cases["X1"]["status"], live.REFUSED)
        self.assertIn("moderates only the test viewer", cases["X1"]["reason"])
        self.assertEqual(cases["X2"]["status"], live.REFUSED)
        self.assertIn("moderates only the test broadcaster's channel", cases["X2"]["reason"])
        # X1 and X2 were refused before asking; X3 (the accounts, then the case) only at its ban.
        self.assertEqual(len(self.prompts), 2)
        self.assertEqual(cases["X3"]["status"], live.REFUSED)
        self.assertEqual([r for r in self.fake.requests() if r[1] == "/moderation/bans"], [])

    def test_a_case_cant_do_more_than_it_declares(self):
        sneaky = live.Case("X4", 3, "Read-only, it says", live.AUTO, live.READ, fake=True,
                           steps=[live.Run("chat", message="hello")])
        self.assertEqual(len(live.check_definitions([sneaky])), 1)
        self.assertEqual(live.check_definitions(live.cases()), [])
        h = self.harness()
        with self.assertRaisesRegex(live.Refusal, "declared read"):
            h.preflight(sneaky)
        h.confirmed = live.READ
        with self.assertRaisesRegex(live.Refusal, "wasn't confirmed"):
            h.allow("chat", {"message": "hello"})
        # Every action of the plugin has a known effect.
        actions = {a["id"] for a in twitch_common.manifest()["actions"]}
        self.assertEqual(actions - set(live.ACTION_EFFECT), set())

    def test_a_long_case_set_to_0_minutes_is_skipped(self):
        check = live.skip_when_zero("l10_minutes")
        with self.assertRaisesRegex(live.SkipCase, "--l10-minutes 0"):
            check(types.SimpleNamespace(args=types.SimpleNamespace(l10_minutes=0)))
        self.assertEqual(check(types.SimpleNamespace(args=types.SimpleNamespace(l10_minutes=10))),
                         (live.DONE, "--l10-minutes 10"))
        l10 = next(c for c in live.cases() if c.id == "L10")
        self.assertEqual(l10.steps[0].fn.__qualname__, check.__qualname__)  # L10 starts with it

    def test_made_up_names_are_checked_with_get_users(self):
        for _ in range(200):
            self.assertRegex(live.made_up_login(), r"^[a-z][a-z0-9]{15}$")
        self.sign_in()
        names = iter([V, "Not A Login!", "pdmadeupname0001"])  # taken, invalid, then free
        cases, report = self.run_harness("--only", "G4", auto_yes=True, make_login=lambda: next(names))
        self.assertEqual(cases["G4"]["status"], live.PASS, cases["G4"])
        self.assertEqual(report["made_up_name"], "pdmadeupname0001")
        requests = self.fake.requests()
        logins = [(i, r[2].get("login")) for i, r in enumerate(requests) if r[:2] == ("GET", "/users")]
        taken = next(i for i, login in logins if login == V)
        made_up = next(i for i, login in logins if login == "pdmadeupname0001")
        first_ban = next(i for i, r in enumerate(requests) if r[:2] == ("POST", "/moderation/bans"))
        self.assertLess(taken, made_up)
        self.assertLess(made_up, first_ban)  # checked before anything was sent
        self.assertFalse(any(r[2].get("login") == "not a login!" for r in requests))
        self.assertIn("made-up name someviewer belongs to an account; trying another", self.out.getvalue())
        # Every made-up name taken: G4 doesn't run, and nobody is banned.
        self.fake.state["requests"].clear()
        cases, _ = self.run_harness("--only", "G4", auto_yes=True, make_login=lambda: V)
        self.assertEqual(cases["G4"]["status"], live.SKIPPED)
        self.assertIn("every made-up name belonged to an account", cases["G4"]["reason"])
        self.assertEqual([r for r in self.fake.requests() if r[1] == "/moderation/bans"], [])

    def test_test_folder_is_never_the_real_one(self):
        self.assertIsNotNone(twitch_common.folder_problem(os.path.join(twitch_common.REPO, "tmp-test-folder")))
        for root in twitch_common.real_data_roots():
            self.assertIsNotNone(twitch_common.folder_problem(os.path.join(root, "config", "plugin-data")), root)
        self.assertIsNotNone(twitch_common.folder_problem(os.path.expanduser("~")))
        fresh = os.path.join(self.tmp, "fresh")
        self.assertIsNone(twitch_common.folder_problem(fresh))
        twitch_common.prepare_test_folder(fresh)
        self.assertIsNone(twitch_common.folder_problem(fresh))  # its own folder again: fine
        other = os.path.join(self.tmp, "someone-elses")
        os.makedirs(other)
        open(os.path.join(other, "notes.txt"), "w").close()
        self.assertIn("already has files", twitch_common.folder_problem(other))

    def test_a_posix_test_folder_must_be_the_users_own(self):
        def st(uid, mode):
            return types.SimpleNamespace(st_uid=uid, st_mode=stat.S_IFDIR | mode)
        problem = twitch_common.ownership_problem
        self.assertIsNone(problem("/tmp/x", st(1000, 0o700), 1000))
        self.assertIsNone(problem("/tmp/x", st(1000, 0o755), 1000))  # readable, but only the user writes
        self.assertIn("belongs to another user", problem("/tmp/x", st(1001, 0o700), 1000))
        self.assertIn("written to by other users", problem("/tmp/x", st(1000, 0o777), 1000))
        self.assertIn("written to by other users", problem("/tmp/x", st(1000, 0o720), 1000))

    @unittest.skipUnless(os.name == "posix", "POSIX file modes")
    def test_a_posix_test_folder_is_made_private(self):
        fresh = os.path.join(self.tmp, "private")
        twitch_common.prepare_test_folder(fresh)
        self.assertEqual(stat.S_IMODE(os.stat(fresh).st_mode), 0o700)
        os.chmod(fresh, 0o777)
        self.assertIn("other users", twitch_common.folder_problem(fresh))
        os.chmod(fresh, 0o700)
        link = os.path.join(self.tmp, "link")
        os.symlink(fresh, link)
        self.assertIn("symbolic link", twitch_common.folder_problem(link))

    def test_reports_and_traces_stay_out_of_the_repo_and_the_real_folder(self):
        self.assertIsNotNone(twitch_common.location_problem(os.path.join(twitch_common.REPO, "report.json")))
        for root in twitch_common.real_data_roots():
            self.assertIsNotNone(twitch_common.location_problem(os.path.join(root, "trace.jsonl")), root)
        self.assertIsNone(twitch_common.location_problem(os.path.join(self.tmp, "report.json")))
        target = os.path.join(HERE, "should-not-exist.json")
        for flag in ("--report", "--trace"):
            err = io.StringIO()
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
                code = live.main(["--dry-run-fake", "--only", "A1", flag, target], input_fn=lambda p: self.fail(p))
            self.assertEqual(code, 2, flag)
            self.assertIn("inside the repository", err.getvalue())
            self.assertFalse(os.path.exists(target))

    def test_the_main_channel_is_never_moderated_or_raided(self):
        h = self.harness("--main-channel", T)
        never = [("ban", {"user": V}), ("unban", {"user": V}), ("warn", {"user": V}), ("vip", {"user": V}),
                 ("mod", {"user": V}), ("chat-mode", {"mode": "emote"}), ("shield", {}), ("automod", {"level": "1"}),
                 ("clear-chat", {}), ("raid", {"channel": R})]
        for action, settings in never:
            self.assertIn("the main channel", h.gate(action, settings) or "", action)
        for action, settings in [("chat", {}), ("set-info", {"title": "x"}), ("shoutout", {"channel": R}),
                                 ("cancel-raid", {}), ("announce", {})]:
            self.assertIsNone(h.gate(action, settings), action)
        # Signed in as the main channel counts too, whatever --broadcaster says.
        h = self.harness("--main-channel", "mainchannel")
        self.assertIsNone(h.gate("ban", {"user": V}))
        h.signed_in_login = lambda: "mainchannel"
        self.assertIn("the main channel", h.gate("ban", {"user": V}))
        # A whole run: refused before asking, and nothing sent.
        self.sign_in()
        cases, _ = self.run_harness("--main-channel", T, "--only", "G2,F3,F4,G6,C7", auto_yes=True)
        for cid in ("G2", "F3", "F4", "G6"):
            self.assertEqual(cases[cid]["status"], live.REFUSED, cid)
            self.assertIn("the main channel", cases[cid]["reason"])
        self.assertEqual(cases["C7"]["status"], live.PASS)
        self.assertEqual([r[:2] for r in self.changes()], [("PATCH", "/channels")] * 4)
        # By hand too: F2, G9 and H3 are refused before their steps are shown, and nothing
        # is sent (not even H3's cancel-raid).
        by_hand = [next(c for c in live.cases() if c.id == cid) for cid in ("F2", "G9", "H3")]
        for case in by_hand:
            case.fake = True
        self.fake.state["requests"].clear()
        h = self.harness("--main-channel", T, answers=["yes"] * 6)
        h.person = True  # a fake run has none
        h.run(by_hand)
        with open(h.report_path, encoding="utf-8") as f:
            refused = {c["id"]: c for c in json.load(f)["cases"]}
        for cid in ("F2", "G9", "H3"):
            self.assertEqual(refused[cid]["status"], live.REFUSED, cid)
            self.assertIn("by hand: ", refused[cid]["reason"])
            self.assertIn("the main channel", refused[cid]["reason"])
        self.assertNotIn("By hand:", self.out.getvalue())
        self.assertEqual(self.prompts, [])
        self.assertEqual(self.changes(), [])
        # And the main channel can't be the account moderation or raids aim at.
        self.assertIn("main channel", live.check_accounts(self.args("--main-channel", V)))
        self.assertIn("main channel", live.check_accounts(self.args("--main-channel", R)))

    def test_no_skip_affiliate_runs_other_risky_cases_only_by_name(self):
        self.sign_in()
        cases, _ = self.run_harness("--no-skip-affiliate", "--only", "B3,C", auto_yes=True)
        self.assertEqual(cases["B3"]["status"], live.PASS)  # nothing public: runs
        for cid in ("C1", "C2", "C3", "C4", "C5", "C6", "C7"):
            self.assertEqual(cases[cid]["status"], live.REFUSED, cid)
            self.assertIn(f"runs only when --only names it (--only {cid})", cases[cid]["reason"])
        self.assertEqual(self.changes(), [])
        cases, _ = self.run_harness("--no-skip-affiliate", "--only", "C7", auto_yes=True)
        self.assertEqual(cases["C7"]["status"], live.PASS)

    def test_a_wrong_account_is_signed_out_at_once(self):
        self.fake.state["signed_in_as"] = fake_twitch.OTHER  # approved in the wrong browser profile
        cases, report = self.run_harness("--only", "A3")
        self.assertEqual(cases["A3"]["status"], live.FAIL)
        text = json.dumps(cases["A3"])
        self.assertIn(f"signed in as {R}, not the test broadcaster {T}; signed out at once", text)
        self.assertEqual(self.fake.state["revoked"], ["access-1"])  # that account's token ended at Twitch
        self.assertFalse(os.path.exists(self.tokens_path()))
        self.assertIsNone(report["signed_in_as"])
        self.assertEqual(report["wrong_account_signed_out"], R)
        self.assertEqual(self.changes(), [])

    def test_a_sign_in_not_approved_in_time_is_signed_out(self):
        def never_approves():
            self.fake.state.update(device_used=False, device_polls=0, pending_polls=10 ** 6)
        h = self.harness("--only", "A3", "--sign-in-minutes", "0.05")
        with mock.patch.object(self.fake, "approve_next_sign_in", never_approves):
            h.run()
        with open(h.report_path, encoding="utf-8") as f:
            a3 = json.load(f)["cases"][0]
        self.assertEqual(a3["status"], live.FAIL)
        self.assertIn("no approval within 0.05 minutes; signed out at once", json.dumps(a3))
        # The plugin's attempt was cancelled: a late approval can't sign anyone in.
        self.assertIn("Twitch sign-in: cancelled by signing out", [m for _, _, m in h.plugin.logs])
        self.assertFalse(os.path.exists(self.tokens_path()))

    def test_a_cases_own_sign_in_is_tidied_up_too(self):
        # A5 approved in the wrong browser profile: that token is revoked at once, as in A3.
        self.sign_in()
        self.fake.state["signed_in_as"] = fake_twitch.OTHER
        cases, report = self.run_harness("--only", "A5")
        self.assertEqual(cases["A5"]["status"], live.FAIL)
        self.assertIn(f"signed in as {R}, not the test broadcaster {T}; signed out at once", json.dumps(cases["A5"]))
        self.assertFalse(os.path.exists(self.tokens_path()))
        self.assertIsNone(report["signed_in_as"])
        self.assertEqual(report["wrong_account_signed_out"], R)
        # A4 with nobody answering: the attempt still polling is cancelled, so a late
        # approval can't swap the sign-in later in the run.
        self.fake.state["signed_in_as"] = None
        a4 = next(c for c in live.cases() if c.id == "A4")
        a4.fake = True
        settle = live.settle_sign_in
        # The same when the person types skip at the prompt instead (the case is then
        # "pass", for the key's answer, with what stopped it under "incomplete").
        for answer, status, said in (("", live.FAIL, "no answer within 0.5 s; signed out at once"),
                                     ("skip", live.PASS, '"incomplete": "skipped by the person; signed out at once')):
            self.sign_in()
            self.fake.state.update(device_used=False, device_polls=0, pending_polls=10 ** 6)
            with mock.patch.object(live, "settle_sign_in", lambda h, mark, seconds: settle(h, mark, 0.5)):
                h = self.harness(answers=[answer])
                h.person = True  # a fake run has none; this one presses Enter, or types skip
                h.run([a4])
            with open(h.report_path, encoding="utf-8") as f:
                case = json.load(f)["cases"][0]
            self.assertEqual(case["status"], status, answer)
            self.assertIn(said, json.dumps(case))
            self.assertIn("Twitch sign-in: cancelled by signing out", [m for _, _, m in h.plugin.logs])
            self.assertFalse(os.path.exists(self.tokens_path()))
        # A15 with no approval in time: its attempt is cancelled too.
        self.sign_in()
        a15 = next(c for c in live.cases() if c.id == "A15")
        a15.fake = True
        self.fake.state.update(device_used=False, device_polls=0, pending_polls=10 ** 6)
        h = self.harness("--sign-in-minutes", "0.01")
        h.person = True
        h.run([a15])
        with open(h.report_path, encoding="utf-8") as f:
            case = json.load(f)["cases"][0]
        self.assertEqual(case["status"], live.FAIL)
        self.assertIn("no approval arrived; signed out at once", json.dumps(case))
        self.assertIn("Twitch sign-in: cancelled by signing out", [m for _, _, m in h.plugin.logs])
        self.assertFalse(os.path.exists(self.tokens_path()))

    def test_a4_compares_the_sign_in_after_the_check_at_start(self):
        # The check at start refreshes an expired token: a Deny must still keep the sign-in.
        self.sign_in()
        with open(self.tokens_path(), encoding="utf-8") as f:
            tokens = json.load(f)
        tokens["expires_at"] = time.time() - 60
        with open(self.tokens_path(), "w", encoding="utf-8") as f:
            json.dump(tokens, f)
        self.fake.state["deny"] = True
        a4 = next(c for c in live.cases() if c.id == "A4")
        a4.fake = True
        h = self.harness(answers=[""])
        h.person = True  # a fake run has none; this one presses Enter after "pressing Deny"
        h.run([a4])
        with open(h.report_path, encoding="utf-8") as f:
            case = json.load(f)["cases"][0]
        self.assertEqual(case["status"], live.PASS, case)
        self.assertIn("Twitch refused the sign-in (access_denied); the earlier sign-in kept", json.dumps(case))
        self.assertIn("Twitch sign-in refreshed", [m for _, _, m in h.plugin.logs])

    def test_cases_a_person_runs_check_the_test_accounts_first(self):
        # F2 and G9 are only Guided steps, but they name V and R: a typo there would have a
        # person moderate a stranger.
        self.sign_in()
        f2, g9 = (next(c for c in live.cases() if c.id == cid) for cid in ("F2", "G9"))
        f2.fake = g9.fake = True

        def run(cases, answers):
            h = self.harness(answers=answers)
            h.person = True  # a fake run has none
            h.run(cases)
            with open(h.report_path, encoding="utf-8") as f:
                return {c["id"]: c for c in json.load(f)["cases"]}

        cases = run([f2, g9], ["no"])
        for cid in ("F2", "G9"):
            self.assertEqual(cases[cid]["status"], live.REFUSED, cid)
            self.assertIn("the test accounts weren't confirmed", cases[cid]["reason"])
        self.assertEqual(len(self.prompts), 1)
        self.assertTrue(self.prompts[0].strip().startswith(ACCOUNTS_PROMPT))
        self.prompts.clear()
        cases = run([f2], ["yes", "p", "worked as moderator"])
        self.assertEqual(cases["F2"]["status"], live.PASS)
        self.assertTrue(self.prompts[0].strip().startswith(ACCOUNTS_PROMPT))
        self.assertTrue(self.prompts[1].strip().startswith("Result?"))
        self.assertEqual(self.changes(), [])
        # And Twitch must confirm the token is T's, whatever tokens.json says.
        self.prompts.clear()
        self.fake.state["signed_in_as"] = fake_twitch.OTHER
        cases = run([f2], ["yes", "p", "note"])
        self.assertEqual(cases["F2"]["status"], live.REFUSED)
        self.assertIn(f"Twitch says the token belongs to {R}", cases["F2"]["reason"])
        self.assertEqual(self.prompts, [])

    def test_no_skip_affiliate_needs_the_main_channel(self):
        err = io.StringIO()
        # Should the check go, the run still mustn't start against the real Twitch.
        with contextlib.redirect_stderr(err), mock.patch.object(live, "Harness") as harness:
            code = live.main(["--broadcaster", T, "--client-id", "x", "--no-skip-affiliate", "--only", "I",
                              "--data-dir", os.path.join(self.tmp, "test-folder")], input_fn=lambda p: self.fail(p))
        self.assertEqual(code, 2)
        self.assertIn("--no-skip-affiliate needs --main-channel", err.getvalue())
        harness.assert_not_called()

    def test_the_test_accounts_are_checked_with_twitch(self):
        self.sign_in()
        # A viewer Twitch doesn't know (a typo): refused before anything is sent.
        cases, _ = self.run_harness("--only", "C7", viewer="pdnosuchviewer01", answers=["yes", "yes"])
        self.assertEqual(cases["C7"]["status"], live.REFUSED)
        self.assertIn("Twitch has no account called --viewer pdnosuchviewer01", cases["C7"]["reason"])
        # V doesn't follow T.
        self.fake.state["follows_me"] = set()
        cases, _ = self.run_harness("--only", "C7", answers=["yes", "yes"])
        self.assertEqual(cases["C7"]["status"], live.REFUSED)
        self.assertIn(f"the test viewer {V} doesn't follow {T}", cases["C7"]["reason"])
        self.assertEqual(self.prompts, [])
        self.assertEqual(self.changes(), [])
        # Shown with IDs and creation dates; anything but yes refuses every risky case after it.
        self.fake.state["follows_me"] = {fake_twitch.VIEWER["id"]}
        cases, _ = self.run_harness("--only", "C7,C3", answers=["no", "yes", "yes"])
        for cid in ("C7", "C3"):
            self.assertEqual(cases[cid]["status"], live.REFUSED, cid)
            self.assertIn("the test accounts weren't confirmed", cases[cid]["reason"])
        self.assertEqual(len(self.prompts), 1)
        self.assertTrue(self.prompts[0].strip().startswith(ACCOUNTS_PROMPT))
        out = self.out.getvalue()
        self.assertIn(f"--viewer {V}: id {fake_twitch.VIEWER['id']}, created 2015-06-01, follows {T} since 2026-10-01",
                      out)
        self.assertIn(f"--raid-target {R}: id {fake_twitch.OTHER['id']}, created 2015-06-01", out)
        self.assertEqual(self.changes(), [])
        # Confirmed once; each case still asks its own yes.
        self.prompts.clear()
        cases, report = self.run_harness("--only", "C7,C3", answers=["yes", "yes", "yes"])
        self.assertEqual((cases["C7"]["status"], cases["C3"]["status"]), (live.PASS, live.PASS))
        self.assertEqual(len(self.prompts), 3)
        self.assertTrue(report["accounts_confirmed"])
        follows = [r[2] for r in self.fake.requests("GET", "/channels/followers") if "user_id" in r[2]]
        self.assertIn({"broadcaster_id": fake_twitch.ME["id"], "user_id": fake_twitch.VIEWER["id"]}, follows)

    def test_a_case_that_stops_early_is_undone(self):
        self.sign_in()

        def ctrl_c(h):
            raise KeyboardInterrupt

        case = live.Case("X6", 9, "Changes, then Ctrl+C", live.SEMI, live.MODERATION, needs={"viewer", "raid_target"},
                         fake=True, steps=[
                             live.Run("ban", user="{V}", duration="0"),
                             live.Run("mod", user="{V}", change="add"),
                             live.Run("vip", user="{V}", change="add"),
                             live.Run("chat-mode", mode="emote", state="on"),
                             live.Run("chat-mode", mode="slow", state="on", duration="3"),
                             live.Run("chat-mode", mode="slow", state="off"),  # put back by the case itself
                             live.Run("shield", state="on"),
                             live.Run("raid", channel="{R}"),
                             live.Custom(ctrl_c, "Ctrl+C")])
        cases, report = self.run_harness(cases=[case], auto_yes=True)
        self.assertTrue(report["interrupted"])
        undone = [s["step"] for s in cases["X6"]["steps"] if s["step"].startswith("undo: ")]
        self.assertEqual(undone, [
            "undo: run cancel-raid",
            "undo: run shield (state='off')",
            "undo: run chat-mode (mode='emote', state='off')",
            f"undo: run vip (user='{V}', change='remove')",
            f"undo: run mod (user='{V}', change='remove')",
            f"undo: run unban (user='{V}')"])
        self.assertNotIn("not_undone", cases["X6"])
        st = self.fake.state
        self.assertEqual(st["roles"]["/moderation/moderators"], set())
        self.assertEqual(st["roles"]["/channels/vips"], set())
        self.assertFalse(st["shield"])
        self.assertFalse(st["chat_settings"][fake_twitch.ME["id"]]["emote_mode"])
        moderation = [(m, p) for m, p, _, _ in st["moderation"]]
        self.assertIn(("DELETE", "/moderation/bans"), moderation)
        self.assertIn(("DELETE", "/raids"), moderation)
        slow_off = [b for m, p, _, b in self.fake.requests("PATCH", "/chat/settings") if b == {"slow_mode": False}]
        self.assertEqual(len(slow_off), 1)  # not undone twice
        # An undo goes through the same checks: on the main channel, nothing is sent.
        h = self.harness("--main-channel", T)
        h.plugin = mock.Mock(exited=False)
        h.confirmed = live.MODERATION
        h.pending_undo = {("ban", V): ("unban", {"user": V})}
        h.run_undo(case)
        self.assertEqual(h.case_steps[-1]["status"], live.UNDO_FAILED)
        self.assertIn("refused", h.case_steps[-1]["detail"])
        h.plugin.run.assert_not_called()

    def test_a_harness_error_is_redacted(self):
        self.sign_in()

        def boom(h):
            raise RuntimeError("the token was " + h.tokens()["access_token"])

        case = live.Case("X7", 3, "A harness error", live.AUTO, live.READ, fake=True, steps=[live.Custom(boom, "boom")])
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            cases, report = self.run_harness(cases=[case])
        self.assertEqual(cases["X7"]["status"], live.FAIL)
        self.assertIn("Traceback", err.getvalue())
        self.assertIn("the token was [redacted]", err.getvalue())
        for secret in SECRETS:
            self.assertNotIn(secret, err.getvalue())
            self.assertNotIn(secret, json.dumps(report))


class DryRunTest(WithFake):
    def test_dry_run_against_the_fakes(self):
        report_path = os.path.join(self.tmp, "dry-report.json")
        trace_path = os.path.join(self.tmp, "dry-trace.jsonl")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = live.main(["--dry-run-fake", "--only", "A1,A2,A3,A13,B1,B3,E4,C7,E1,L8",
                              "--report", report_path, "--trace", trace_path],
                             input_fn=lambda p: self.fail("asked: " + p))
        self.assertEqual(code, 0, out.getvalue())
        with open(report_path, encoding="utf-8") as f:
            report = json.load(f)
        statuses = {c["id"]: c["status"] for c in report["cases"]}
        self.assertEqual(statuses, dict.fromkeys(["A1", "A2", "A3", "A13", "B1", "B3", "E4", "C7", "E1", "L8"],
                                                 live.PASS))
        self.assertEqual(report["signed_in_as"], T)
        self.assertIn("[dry run against the fakes: answering yes]", out.getvalue())
        with open(trace_path, encoding="utf-8") as f:
            trace = f.read()
        for text, where in ((out.getvalue(), "output"), (json.dumps(report), "report"), (trace, "trace")):
            for secret in SECRETS:
                self.assertNotIn(secret, text, where)
        self.assertNotIn("ABCDEFGH", trace)  # the user code, and the device code behind it
        self.assertNotIn("ABCDEFGH", json.dumps(report))
        self.assertIn("enter [redacted]", json.dumps(report))
        self.assertIn("enter ABCDEFGH", out.getvalue())  # the person still sees it
        self.assertIn('"Authorization": "Bearer [redacted]"', trace)
        self.assertIn(f'"Client-id": "{fake_twitch.CLIENT_ID}"', trace)
        summary = twitch_trace.analyse(twitch_trace.load(trace_path))
        self.assertGreater(summary["helix_requests"], 10)
        self.assertLess(summary["helix_busiest_minute"], 800)
        self.assertGreaterEqual(summary["eventsub_connects"], 1)
        fixtures = twitch_trace.to_fixtures(twitch_trace.load(trace_path), trace_path)
        text = json.dumps(fixtures)
        for name in (T, V, R, fake_twitch.ME["id"], fake_twitch.VIEWER["id"]):
            self.assertNotIn(name, text)
        streams = next(f for f in fixtures["fixtures"] if f["name"] == "GET /helix/streams 200")
        self.assertIs(twitch_trace.find_fixture(fixtures, "GET", "/helix/streams", streams["request"]["query"]), streams)


class TraceTest(unittest.TestCase):
    def test_redactor(self):
        r = twitch_common.Redactor()
        record = {
            "request_headers": {"Authorization": "Bearer tok3nvalue0123456789abcdefghij", "Client-Id": "myclientid"},
            "request_body": {"client_id": "myclientid", "token": "tok3nvalue0123456789abcdefghij",
                             "grant_type": "refresh_token", "refresh_token": "r3fresh0123456789"},
            "response_body": {"access_token": "acc3ss0123456789", "device_code": "d3vice0123456789",
                              "user_code": "WXYZ1234", "message": "Invalid OAuth token",
                              "verification_uri": "https://www.twitch.tv/activate?public=true&device-code=WXYZ1234"},
            "error": "the server said acc3ss0123456789 was bad; OAuth n3wtoken987654321 too",
        }
        clean = json.dumps(r.scrub(record))
        for secret in ("tok3nvalue", "r3fresh", "acc3ss", "d3vice", "WXYZ1234", "n3wtoken"):
            self.assertNotIn(secret, clean)
        self.assertIn('"Authorization": "Bearer [redacted]"', clean)
        self.assertIn('"Client-Id": "myclientid"', clean)
        self.assertIn('"client_id": "myclientid"', clean)
        self.assertIn('"grant_type": "refresh_token"', clean)
        self.assertIn("Invalid OAuth token", clean)  # Twitch's wording stays

    def test_fixtures(self):
        records = [
            {"seq": 1, "t": 1791900000.0, "kind": "start"},
            {"seq": 2, "t": 1791900001.0, "kind": "http", "method": "GET", "path": "/helix/users",
             "query": {"login": ["SomeStreamer", "someviewer"]}, "request_body": None, "status": 200,
             "response_headers": {"Ratelimit-Remaining": "799", "Set-Cookie": "x", "Content-Type": "application/json"},
             "response_body": {"data": [
                 {"id": "4242", "login": "somestreamer", "display_name": "SomeStreamer", "email": "x@example.com",
                  "description": "my bio", "profile_image_url": "https://static-cdn.jtvnw.net/someone.png"},
                 {"id": "5151", "login": "someviewer", "display_name": "SomeViewer"}]}},
            {"seq": 3, "t": 1791900002.0, "kind": "http", "method": "GET", "path": "/helix/chat/chatters",
             "query": {"broadcaster_id": ["5151"], "moderator_id": ["4242"]}, "request_body": None, "status": 403,
             "response_headers": {}, "response_body": {"error": "Forbidden", "status": 403,
                                                       "message": "The user in moderator_id must have moderator privileges"}},
            {"seq": 4, "t": 1791900003.0, "kind": "http", "method": "GET", "path": "/helix/chat/chatters",
             "query": {"broadcaster_id": ["5151"], "moderator_id": ["4242"]}, "request_body": None, "status": 403,
             "response_headers": {}, "response_body": {"error": "Forbidden", "status": 403,
                                                       "message": "The user in moderator_id must have moderator privileges"}},
            {"seq": 5, "t": 1791900004.0, "kind": "http", "method": "GET", "path": "/helix/games",
             "query": {"name": ["Just Chatting"]}, "request_body": None, "status": 200, "response_headers": {},
             "response_body": {"data": [{"id": "509658", "name": "Just Chatting"}]}},
            {"seq": 6, "t": 1791900005.0, "kind": "ws", "event": "message", "message_type": "notification",
             "subscription_type": "channel.raid", "subscription_version": "1",
             "condition": {"to_broadcaster_user_id": "4242"},
             "event_payload": {"from_broadcaster_user_login": "someviewer", "from_broadcaster_user_name": "SomeViewer",
                               "from_broadcaster_user_id": "5151", "viewers": 7}},
            {"seq": 7, "t": 1791900006.0, "kind": "http", "method": "GET", "path": "/helix/streams",
             "query": {"user_id": ["4242"]}, "request_body": None, "status": 200, "response_headers": {},
             "response_body": {"data": [{
                 "id": "40001", "user_id": "4242", "user_login": "somestreamer", "user_name": "SomeStreamer",
                 "type": "live", "title": "Testing with friends", "viewer_count": 3,
                 "thumbnail_url":
                     "https://static-cdn.jtvnw.net/previews-ttv/live_user_somestreamer-{width}x{height}.jpg",
                 "vod_note": "https://static-cdn.jtvnw.net/cf_vods/abc123_somestreamer_40001/thumb.jpg"}]}},
        ]
        data = twitch_trace.to_fixtures(records, "trace.jsonl")
        self.assertEqual((data["format"], data["recorded"], data["source"]),
                         ("perritodeck-twitch-fixtures/1", "2026-10-13", "trace.jsonl"))
        self.assertEqual(len(data["fixtures"]), 4)  # the second 403 is the same answer
        text = json.dumps(data)
        for real in ("somestreamer", "SomeStreamer", "someviewer", "4242", "5151", "x@example.com", "my bio",
                     "someone.png", "Set-Cookie", "live_user_"):
            self.assertNotIn(real, text)
        stream = data["fixtures"][3]["response"]["body"]["data"][0]
        self.assertEqual(stream["thumbnail_url"], "https://example.invalid/thumbnail-{width}x{height}.jpg")
        self.assertEqual(stream["vod_note"], "https://static-cdn.jtvnw.net/cf_vods/abc123_user1_40001/thumb.jpg")
        # A login that only the query has, repeated in Twitch's message: the fixture's name
        # is built from the message, so the query's names are learned first.
        refused = [{"seq": 1, "t": 1791900000.0, "kind": "http", "method": "GET", "path": "/helix/users",
                    "query": {"login": ["realperson_x"]}, "request_body": None, "status": 400,
                    "response_headers": {}, "response_body": {
                        "error": "Bad Request", "status": 400,
                        "message": "Invalid login names, emails or IDs in request: realperson_x"}}]
        only = twitch_trace.to_fixtures(refused, "trace.jsonl")
        self.assertNotIn("realperson_x", json.dumps(only))
        self.assertEqual(only["fixtures"][0]["name"],
                         "GET /helix/users 400 (Invalid login names, emails or IDs in request: user1)")
        # A gifter's and a clip creator's IDs are people's IDs. Hosts in addresses stay, and
        # so does Twitch's own channel (the read-only cases' default), so "Twitch" in
        # Twitch's messages stays too.
        def http(seq, method, path, query, body):
            return {"seq": seq, "t": 1791900000.0 + seq, "kind": "http", "method": method, "path": path,
                    "query": query, "request_body": None, "status": 200, "response_headers": {}, "response_body": body}
        more = twitch_trace.to_fixtures([
            http(1, "GET", "/helix/users", {"login": ["twitch"]},
                 {"data": [{"id": "12826", "login": "twitch", "display_name": "Twitch"}]}),
            http(2, "POST", "/oauth2/device", {},
                 {"device_code": "[redacted]", "verification_uri": "https://www.twitch.tv/activate?public=true"}),
            http(3, "GET", "/helix/subscriptions", {"broadcaster_id": ["4242"]}, {"data": [{
                "broadcaster_id": "4242", "broadcaster_login": "somestreamer", "user_id": "7777", "user_login": "subber",
                "user_name": "Subber", "gifter_id": "8888", "gifter_login": "giftgiver", "gifter_name": "GiftGiver",
                "is_gift": True}]}),
            http(4, "GET", "/helix/clips", {"id": ["Slug"]}, {"data": [{
                "id": "Slug", "creator_id": "9999", "creator_name": "Clipper", "game_id": "509658",
                "edit_url": "https://clips.twitch.tv/Slug/edit", "title": "Twitch moment"}]}),
        ], "trace.jsonl")
        more_text = json.dumps(more)
        for real in ("4242", "7777", "8888", "9999", "somestreamer", "subber", "giftgiver", "GiftGiver", "Clipper"):
            self.assertNotIn(real, more_text)
        self.assertIn("https://www.twitch.tv/activate?public=true", more_text)
        self.assertIn("https://clips.twitch.tv/Slug/edit", more_text)
        self.assertIn('"game_id": "509658"', more_text)  # a game, not a person
        self.assertIn('"title": "Twitch moment"', more_text)
        self.assertEqual(more["fixtures"][0]["response"]["body"]["data"][0]["login"], "twitch")
        defaults = live.parse_args([])
        self.assertLessEqual({defaults.public_channel, *defaults.b8_channels.split(",")}, twitch_trace.TWITCH_CHANNELS)
        # What viewers write can name anyone, so it's replaced whole; and a host that isn't
        # Twitch's can carry a login.
        def event(seq, sub_type, payload):
            return {"seq": seq, "t": 1791900000.0 + seq, "kind": "ws", "event": "message",
                    "message_type": "notification", "subscription_type": sub_type, "subscription_version": "1",
                    "condition": {"broadcaster_user_id": "4242"}, "event_payload": payload}
        written = twitch_trace.to_fixtures([
            event(1, "channel.cheer", {"user_login": "cheerer9", "user_name": "Cheerer9", "user_id": "5001",
                                       "bits": 100, "message": "Cheer100 art at https://cheerer9.carrd.co, hi @pal99"}),
            event(2, "channel.subscription.message", {"user_login": "resubber", "user_id": "5002", "message": {
                "text": "thanks @pal99", "emotes": [{"begin": 0, "end": 5, "id": "25"}]}, "cumulative_months": 7}),
            event(3, "channel.channel_points_custom_reward_redemption.add", {
                "user_login": "redeemer", "user_id": "5003", "user_input": "play a song for @pal99",
                "reward": {"id": "r1", "title": "Song request", "cost": 50}}),
            http(4, "GET", "/helix/channels", {"broadcaster_id": ["4242"]}, {"data": [{
                "broadcaster_id": "4242", "broadcaster_login": "hostperson", "broadcaster_name": "HostPerson",
                "title": "my site: https://hostperson.carrd.co and https://www.twitch.tv/hostperson"}]}),
        ], "trace.jsonl")
        written_text = json.dumps(written)
        for real in ("cheerer9", "resubber", "redeemer", "pal99", "hostperson", "HostPerson", "5001", "5002", "5003"):
            self.assertNotIn(real, written_text)
        cheer, resub, redemption = (e["event"] for e in written["events"])
        self.assertEqual(cheer["message"], twitch_trace.VIEWER_TEXT)
        self.assertEqual(resub["message"], {"text": twitch_trace.VIEWER_TEXT, "emotes": []})
        self.assertEqual(redemption["user_input"], twitch_trace.VIEWER_TEXT)
        self.assertEqual(redemption["reward"]["title"], "Song request")  # the broadcaster's own words stay
        title = written["fixtures"][0]["response"]["body"]["data"][0]["title"]
        self.assertRegex(title, r"^my site: https://user\d+\.carrd\.co and https://www\.twitch\.tv/user\d+$")
        anon = twitch_trace.Anonymiser()
        anon.name("somestreamer")
        self.assertEqual(anon.text("live_user_somestreamer-{width}x{height}.jpg, SomeStreamer, somestreamers"),
                         "live_user_user1-{width}x{height}.jpg, user1, somestreamers")
        users = data["fixtures"][0]
        self.assertEqual(users["request"]["query"], {"login": ["User1", "user2"]})
        self.assertEqual([u["login"] for u in users["response"]["body"]["data"]], ["user1", "user2"])
        self.assertEqual(users["response"]["body"]["data"][0]["id"], "1000001")
        chatters = data["fixtures"][1]
        self.assertEqual(chatters["request"]["query"], {"broadcaster_id": ["1000002"], "moderator_id": ["1000001"]})
        self.assertIn("Just Chatting", text)  # games aren't people
        self.assertEqual(data["events"], [{"subscription_type": "channel.raid", "subscription_version": "1",
                                           "condition": {"to_broadcaster_user_id": "1000001"},
                                           "event": {"from_broadcaster_user_login": "user2",
                                                     "from_broadcaster_user_name": "User2",
                                                     "from_broadcaster_user_id": "1000002", "viewers": 7}}])
        found = twitch_trace.find_fixture(data, "GET", "/helix/chat/chatters",
                                          {"broadcaster_id": "1000002", "moderator_id": "1000001", "first": "1"})
        self.assertEqual(found["response"]["status"], 403)
        self.assertIsNone(twitch_trace.find_fixture(data, "GET", "/helix/chat/chatters", {"broadcaster_id": "9"}))
        summary = twitch_trace.analyse(records)
        self.assertEqual(summary["errors"][0]["times"], 2)

    def test_run_needs_a_test_folder(self):
        env = {k: v for k, v in os.environ.items() if k != "PERRITODECK_PLUGIN_DATA"}
        r = subprocess.run([sys.executable, os.path.join(HERE, "twitch_trace.py"), "run"], env=env,
                           capture_output=True, text=True, stdin=subprocess.DEVNULL, timeout=60)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("PERRITODECK_PLUGIN_DATA isn't set", r.stderr)


class CleanupTest(WithFake):
    def tokens_file(self, expires_in=3600):
        path = os.path.join(self.tmp, "tokens.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump({"client_id": fake_twitch.CLIENT_ID, "access_token": self.fake.state["access"],
                       "refresh_token": self.fake.state["refresh"], "expires_at": time.time() + expires_in,
                       "login": T, "user_id": fake_twitch.ME["id"]}, f)
        return path

    def cleanup(self, *argv):
        lines = []
        args = twitch_cleanup.parse_args(["--tokens", self.tokens_file(), "--helix-url", self.fake.helix_url,
                                          "--id-url", self.fake.id_url, *argv])
        try:
            n = twitch_cleanup.cleanup(args, lines.append)
        except twitch_cleanup.Problem as e:
            n = e
        return n, "\n".join(lines)

    def test_a_dry_run_then_yes(self):
        n, out = self.cleanup()
        self.assertEqual(n, 1)
        self.assertIn("Would delete", out)
        self.assertIn("'Hydrate!'", out)
        self.assertNotIn("Song request", out)  # made on twitch.tv, not by this app
        self.assertEqual(self.fake.requests("DELETE"), [])
        n, out = self.cleanup("--broadcaster", T, "--match", "nothing like it", "--yes")
        self.assertEqual(n, 0)
        self.assertEqual(self.fake.requests("DELETE"), [])
        n, out = self.cleanup("--broadcaster", R, "--yes")
        self.assertIsInstance(n, twitch_cleanup.Problem)
        self.assertEqual(self.fake.requests("DELETE"), [])
        # --yes without --broadcaster: refused before asking Twitch anything.
        calls = len(self.fake.state["calls"])
        n, out = self.cleanup("--match", "HYDRATE", "--yes")
        self.assertIsInstance(n, twitch_cleanup.Problem)
        self.assertIn("--yes needs --broadcaster", str(n))
        self.assertEqual(len(self.fake.state["calls"]), calls)
        n, out = self.cleanup("--broadcaster", T, "--match", "HYDRATE", "--yes")
        self.assertEqual(n, 1)
        self.assertEqual([(r[0], r[1], r[2].get("id")) for r in self.fake.requests("DELETE")],
                         [("DELETE", "/channel_points/custom_rewards", "r1")])
        self.assertEqual([r["title"] for r in self.fake.state["rewards"]], ["Song request"])
        for secret in SECRETS:
            self.assertNotIn(secret, out)

    def test_an_expired_sign_in_isnt_refreshed(self):
        path = self.tokens_file(expires_in=-10)
        args = twitch_cleanup.parse_args(["--tokens", path, "--helix-url", self.fake.helix_url,
                                          "--id-url", self.fake.id_url, "--broadcaster", T, "--yes"])
        with self.assertRaisesRegex(twitch_cleanup.Problem, "has expired"):
            twitch_cleanup.cleanup(args, lambda line: None)
        self.assertEqual(self.fake.state["calls"], [])

    def test_its_own_sign_in_is_revoked(self):
        self.fake.approve_next_sign_in()
        lines = []
        args = twitch_cleanup.parse_args(["--sign-in", "--client-id", fake_twitch.CLIENT_ID, "--helix-url",
                                          self.fake.helix_url, "--id-url", self.fake.id_url])
        self.assertEqual(twitch_cleanup.cleanup(args, lines.append), 1)
        self.assertIn("enter ABCDEFGH", "\n".join(lines))
        self.assertEqual(self.fake.state["device_scopes"], "channel:manage:redemptions")
        self.assertEqual(self.fake.state["revoked"], ["access-1"])


class LayoutTest(unittest.TestCase):
    @staticmethod
    def plan_section3():
        with open(os.path.join(twitch_common.REPO, "docs", "testing", "twitch.md"), encoding="utf-8") as f:
            return f.read().split("## 3.", 1)[1].split("## 4.", 1)[0]

    @staticmethod
    def documented_differences():
        """The case IDs plugins/tools/README.md lists under "Where the harness differs from
        the plan"."""
        with open(os.path.join(HERE, "README.md"), encoding="utf-8") as f:
            readme = f.read()
        table = readme.split("### Where the harness differs from the plan", 1)[1].split("\n### ", 1)[0]
        ids = set()
        for first_cell in re.findall(r"^\| ([^|]+) \|", table, re.M):
            ids |= set(re.findall(r"\b[A-N]\d+\b", first_cell))
        return ids

    def test_cases_follow_the_plan(self):
        section3 = self.plan_section3()
        ids = re.findall(r"^\| ([A-N]\d+) \|", section3, re.M)
        self.assertEqual(len(ids), len(set(ids)))
        cases = live.cases()
        self.assertEqual([c.id for c in cases if c.id in ids], [c.id for c in cases])
        self.assertEqual(sorted(c.id for c in cases), sorted(ids))
        # The stages of the plan's Order list (ranges like B1–B9; J and K are whole sections),
        # and the cases it names, in its order.
        order = section3.split("### Order", 1)[1].split("**Never", 1)[0]
        stage_of, named = {}, []
        for number, text in re.findall(r"^(\d+)\. (.+?)(?=^\d+\. |\Z)", order, re.M | re.S):
            for a, b in re.findall(r"\b([A-N]\d+)(?:–([A-N]?\d+))?", text):
                letter = a[0]
                last = int(b.lstrip(letter)) if b else int(a[1:])
                for n in range(int(a[1:]), last + 1):
                    stage_of.setdefault(f"{letter}{n}", int(number))
                    if f"{letter}{n}" not in named:
                        named.append(f"{letter}{n}")
        moved = {"M1": 11}  # the soak runs at the end of stage 11, not in the background
        for c in cases:
            want = moved.get(c.id, stage_of.get(c.id, 12 if c.section in live.AFFILIATE_SECTIONS else None))
            self.assertEqual(c.stage, want, c.id)
        self.assertEqual([c.stage for c in cases], sorted(c.stage for c in cases))
        # In the plan's order, but for A8 and M1, which close stage 11 (each documented).
        at_end = ["A8", "M1"]
        documented = self.documented_differences()
        for cid in at_end:
            self.assertIn(cid, documented)
        self.assertEqual([c.id for c in cases if c.id in named and c.id not in at_end],
                         [cid for cid in named if cid not in at_end])
        self.assertEqual([c.id for c in cases if c.stage == 11][-len(at_end):], at_end)

    def test_each_case_runs_as_the_plan_says_or_the_readme_says_why_not(self):
        rows = {}
        for line in self.plan_section3().splitlines():
            m = re.match(r"^\| ([A-N]\d+) \|(.*)\|\s*$", line)
            if m:
                rows[m.group(1)] = [cell.strip() for cell in m.group(2).split("|")]
        documented = self.documented_differences()
        differ = []
        for c in live.cases():
            plan_how = rows[c.id][-1]  # ID | Steps | Expected | Main? | How
            if c.how not in {w.lower() for w in re.findall(r"\b(Auto|Semi|Manual)\b", plan_how)}:
                differ.append(c.id)
                self.assertIn(c.id, documented, f"{c.id} is {c.how} here but '{plan_how}' in the plan: align it, or "
                                                "list it in plugins/tools/README.md")
        self.assertEqual(differ, ["A3", "A4", "A6"])
        # F2 and G9 act in R's channel, where the harness never moderates: a person does them.
        for cid in ("F2", "G9"):
            case = next(c for c in live.cases() if c.id == cid)
            self.assertTrue(all(isinstance(s, live.Guided) for s in case.steps), cid)

    def test_the_catalogue_skips_this_folder(self):
        self.assertFalse(os.path.exists(os.path.join(HERE, "perritodeck-plugin.json")))
        out = tempfile.mkdtemp(prefix="catalogue-")
        self.addCleanup(shutil.rmtree, out, True)
        r = subprocess.run([sys.executable, os.path.join(twitch_common.PLUGINS, "build_catalog.py"), out],
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stderr)
        with open(os.path.join(out, "index.json"), encoding="utf-8") as f:
            index = json.load(f)
        self.assertTrue(index["plugins"])
        self.assertFalse([p for p in index["plugins"] if "tool" in p["id"] or "tool" in p["url"]])
        for name in os.listdir(out):
            self.assertNotIn("twitch_live_test", name)
        # Inside every zip: no file of this folder, by name or by content.
        tools = {}
        for name in os.listdir(HERE):
            path = os.path.join(HERE, name)
            if os.path.isfile(path):
                with open(path, "rb") as f:
                    tools[hashlib.sha256(f.read()).hexdigest()] = name
        names = set(tools.values()) - {"README.md"}  # every plugin has a README.md of its own
        zips = sorted(n for n in os.listdir(out) if n.endswith(".zip"))
        self.assertEqual(len(zips), len(index["plugins"]))
        for zip_name in zips:
            with zipfile.ZipFile(os.path.join(out, zip_name)) as z:
                for info in z.infolist():
                    self.assertNotIn(info.filename.rsplit("/", 1)[-1], names, f"{zip_name}: {info.filename}")
                    self.assertNotIn(hashlib.sha256(z.read(info)).hexdigest(), tools, f"{zip_name}: {info.filename}")


if __name__ == "__main__":
    unittest.main()
