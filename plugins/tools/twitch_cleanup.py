#!/usr/bin/env python3
"""Delete the custom channel points rewards your Twitch app made (docs/testing/twitch.md, J7).

    py -3 plugins/tools/twitch_cleanup.py                        # a dry run: lists them
    py -3 plugins/tools/twitch_cleanup.py --broadcaster T --match "PerritoDeck test" --yes
    py -3 plugins/tools/twitch_cleanup.py --sign-in --broadcaster M --match "PerritoDeck test"
    py -3 plugins/tools/twitch_cleanup.py --dry-run-fake [--yes]  # against the fakes

Twitch lets only the app (client ID) that made a reward delete it, and the dashboard may not
offer to. This lists the rewards with GET /channel_points/custom_rewards?
only_manageable_rewards=true (the ones this client ID made) and, only with --yes, deletes
each with DELETE /channel_points/custom_rewards. Without --yes it changes nothing. --yes
needs --broadcaster: it deletes nothing unless the sign-in belongs to that account (the
harness's tokens.json may hold another account's sign-in than you think).

Every reward the app made is listed, including ones in real use if the plugin manages the
channel's real rewards too: narrow it with --match (part of the title, any case).

The sign-in it uses:
- by default the harness's, from its test folder's tokens.json (--tokens for another
  tokens.json). It never refreshes that token, since a refresh token works only once and
  the plugin keeps using it: if the token has expired, press any Twitch key or run the
  harness, so the plugin refreshes it, and run this again.
- with --sign-in, a sign-in of its own (the device code flow, asking only for
  channel:manage:redemptions): it shows a code, waits for you to approve it in the browser
  profile signed in as the broadcaster, keeps the token in memory only, and revokes it at
  the end.
It never prints a token. Standard library only.
"""

import argparse
import json
import os
import shutil
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from twitch_common import (CLIENT_ID_ENV, DEFAULT_TEST_FOLDER, HELIX_URL, ID_URL, HelixError, Redactor,  # noqa: E402
                           helix, http_json, login_key, message_of, plugin_data, read_tokens)

SCOPE = "channel:manage:redemptions"
DEVICE_GRANT = "urn:ietf:params:oauth:grant-type:device_code"


class Problem(Exception):
    pass


def device_sign_in(client_id, id_url, say, wait_minutes=10):
    """The device code flow for SCOPE; returns the access token (kept in memory only)."""
    status, answer, _ = http_json("POST", id_url + "/oauth2/device", form={"client_id": client_id, "scopes": SCOPE})
    if status != 200 or not answer.get("device_code"):
        raise Problem(f"Twitch refused the sign-in ({status}: {message_of(answer)})")
    device_code, interval = answer["device_code"], float(answer.get("interval") or 5)
    say(f"Sign in as the broadcaster: open {answer.get('verification_uri') or 'https://www.twitch.tv/activate'} "
        f"and enter {answer.get('user_code')}. Waiting up to {wait_minutes:g} minutes...")
    deadline = time.monotonic() + wait_minutes * 60
    while time.monotonic() < deadline:
        time.sleep(max(0.05, interval))
        status, answer, _ = http_json("POST", id_url + "/oauth2/token", form={
            "client_id": client_id, "scopes": SCOPE, "device_code": device_code, "grant_type": DEVICE_GRANT})
        if status == 200 and answer.get("access_token"):
            return answer["access_token"]
        message = message_of(answer)
        if message == "authorization_pending":
            continue
        if message == "slow_down":
            interval += 5
            continue
        raise Problem(f"the sign-in didn't work ({status}: {message})")
    raise Problem("no approval in time")


def revoke(client_id, token, id_url):
    try:
        http_json("POST", id_url + "/oauth2/revoke", form={"client_id": client_id, "token": token})
    except HelixError:
        pass


def cleanup(args, say=print):
    """Returns the number of rewards it deleted (or would delete)."""
    if args.yes and not login_key(args.broadcaster):
        raise Problem("--yes needs --broadcaster <login>: rewards are deleted only from that account's channel, "
                      "and only when the sign-in belongs to it")
    redactor = Redactor()
    own_token = None
    if args.sign_in:
        client_id = (args.client_id or os.environ.get(CLIENT_ID_ENV) or "").strip()
        if not client_id:
            raise Problem(f"--sign-in needs the app's client ID: --client-id or {CLIENT_ID_ENV}")
        token = own_token = device_sign_in(client_id, args.id_url, say, args.sign_in_minutes)
    else:
        tokens = read_tokens(args.tokens)
        if not tokens:
            raise Problem(f"no sign-in in {args.tokens}: run the harness and sign in first, pass --tokens, "
                          "or use --sign-in")
        redactor.learn_tokens(tokens)
        token = tokens["access_token"]
        client_id = (args.client_id or tokens.get("client_id") or "").strip()
        if args.client_id and tokens.get("client_id") and args.client_id != tokens["client_id"]:
            raise Problem("that tokens.json belongs to another client ID than --client-id")
        if float(tokens.get("expires_at") or 0) < time.time() + 60:
            raise Problem("the sign-in in that tokens.json has expired; press any Twitch key (or run the harness) "
                          "so the plugin refreshes it, then run this again")
    redactor.learn(token)
    try:
        status, answer, _ = helix("GET", "/users", token=token, client_id=client_id, base=args.helix_url)
        if status != 200:
            raise Problem(f"Twitch didn't accept the sign-in ({status}: {message_of(answer)})")
        me = (answer.get("data") or [{}])[0]
        login, broadcaster_id = login_key(me.get("login")), me.get("id")
        say(f"Signed in as {login}.")
        if args.broadcaster and login != login_key(args.broadcaster):
            raise Problem(f"signed in as {login}, not {login_key(args.broadcaster)}: nothing deleted")
        status, answer, _ = helix("GET", "/channel_points/custom_rewards",
                                  {"broadcaster_id": broadcaster_id, "only_manageable_rewards": "true"},
                                  token=token, client_id=client_id, base=args.helix_url)
        if status != 200:
            raise Problem(f"Twitch wouldn't list the rewards ({status}: {message_of(answer)})")
        rewards = answer.get("data") or []
        match = (args.match or "").strip().lower()
        chosen = [r for r in rewards if match in (r.get("title") or "").lower()]
        if not chosen:
            say("This app made no rewards on that channel" + (f" with {args.match!r} in the title." if match else "."))
            return 0
        verb = "Deleting" if args.yes else "Would delete (a dry run; --yes deletes them)"
        say(f"{verb} {len(chosen)} reward(s) this app made on {login}'s channel:")
        deleted = 0
        for r in chosen:
            line = f"  {r.get('title')!r} (cost {r.get('cost')}, {'enabled' if r.get('is_enabled') else 'disabled'}, " \
                   f"id {r.get('id')})"
            if not args.yes:
                say(line)
                continue
            status, answer, _ = helix("DELETE", "/channel_points/custom_rewards",
                                      {"broadcaster_id": broadcaster_id, "id": r.get("id")},
                                      token=token, client_id=client_id, base=args.helix_url)
            if status == 204:
                deleted += 1
                say(line + ": deleted")
            else:
                say(line + f": Twitch said {status}: {message_of(answer)}")
        if not args.yes:
            say("Nothing was deleted.")
            return len(chosen)
        say(f"Deleted {deleted} of {len(chosen)}.")
        return deleted
    finally:
        if own_token:
            revoke(client_id, own_token, args.id_url)


def parse_args(argv=None):
    p = argparse.ArgumentParser(description="List, and with --yes delete, the channel points rewards this Twitch "
                                "app made. Without --yes it's a dry run.")
    p.add_argument("--yes", action="store_true", help="really delete them (needs --broadcaster)")
    p.add_argument("--match", help="only rewards with this in the title (any case), e.g. \"PerritoDeck test\"")
    p.add_argument("--broadcaster", help="refuse unless signed in as this login (required with --yes)")
    p.add_argument("--tokens", default=os.path.join(plugin_data(DEFAULT_TEST_FOLDER), "tokens.json"),
                   help="the plugin's tokens.json to use (default: the harness's test folder)")
    p.add_argument("--sign-in", action="store_true", help="sign in with the device code flow instead")
    p.add_argument("--sign-in-minutes", type=float, default=10, help="how long to wait for the approval (10)")
    p.add_argument("--client-id", help=f"the app's client ID (default: the tokens.json's; or {CLIENT_ID_ENV})")
    p.add_argument("--dry-run-fake", action="store_true", help="run against the fake Twitch from the plugin's tests")
    p.add_argument("--helix-url", default=HELIX_URL, help=argparse.SUPPRESS)
    p.add_argument("--id-url", default=ID_URL, help=argparse.SUPPRESS)
    return p.parse_args(argv)


def main(argv=None, say=print):
    args = parse_args(argv)
    fake = folder = None
    if args.dry_run_fake:
        import fake_twitch
        fake = fake_twitch.FakeTwitch()
        args.helix_url, args.id_url = fake.helix_url, fake.id_url
        args.broadcaster = args.broadcaster or fake_twitch.ME["login"]  # the account the fakes sign in as
        if args.sign_in:
            fake.approve_next_sign_in()
            args.client_id = fake_twitch.CLIENT_ID
        else:
            folder = tempfile.mkdtemp(prefix="twitch-cleanup-")
            args.tokens = os.path.join(folder, "tokens.json")
            with open(args.tokens, "w", encoding="utf-8") as f:
                json.dump({"client_id": fake_twitch.CLIENT_ID, "access_token": fake.state["access"],
                           "refresh_token": fake.state["refresh"], "expires_at": time.time() + 3600,
                           "login": fake_twitch.ME["login"], "user_id": fake_twitch.ME["id"]}, f)
        say(f"Against the fakes: the fake channel has {len(fake.state['rewards'])} rewards, "
            f"{sum(1 for r in fake.state['rewards'] if r['mine'])} made by this app.")
    try:
        cleanup(args, say)
        return 0
    except (Problem, HelixError) as e:
        say(f"Stopped: {e}")
        return 1
    finally:
        if fake is not None:
            fake.stop()
        if folder:
            shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
