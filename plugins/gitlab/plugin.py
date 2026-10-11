"""GitLab: watch and run CI/CD pipelines, count open merge requests and issues.

Uses the GitLab REST API v4 with a personal, group or project access token
(PRIVATE-TOKEN header). Works with gitlab.com and self-managed GitLab.
Standard library only.
"""

import json
import os
import re
import ssl
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import webbrowser

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
DEFAULT_URL = "https://gitlab.com"
TIMEOUT = 4  # per request; an action makes at most a few
POLL_SECONDS = 30
GITLAB_COM_POLL_SECONDS = 60  # be gentle with the shared service
REFRESH_AFTER_ACTION = 3.0

PIPELINE_VALUES = ("pipeline", "pipeline_ok", "pipeline_running")
COUNT_VALUES = ("mrs", "issues")
ACTIVE = {"created", "waiting_for_resource", "preparing", "waiting_for_callback", "pending", "running", "canceling"}

_last_poll = {"at": 0.0}


class GitLabError(RuntimeError):
    pass


def config():
    s = plugin.settings or {}
    url = (s.get("url") or "").strip().rstrip("/") or DEFAULT_URL
    if not url.startswith(("https://", "http://")):
        url = "https://" + url
    if url.endswith("/api/v4"):
        url = url[: -len("/api/v4")]
    token = (s.get("token") or "").strip()
    if not token:
        raise GitLabError("set a GitLab access token on the Plugins page")
    return url, token, bool(s.get("insecure"))


def tls_context(insecure):
    ctx = ssl.create_default_context()
    if insecure:
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
    return ctx


def split_target(text):
    """'group/project@branch' -> ('group/project', 'branch'); the branch is optional."""
    text = (text or "").strip().strip("/")
    project, _, ref = text.partition("@")
    project = project.strip().strip("/")
    if not project:
        raise GitLabError("enter the project path, e.g. group/project")
    # A pasted web address works too.
    for prefix in ("https://", "http://"):
        if project.startswith(prefix):
            project = project[len(prefix):].split("/", 1)[-1]
            project = project.split("/-/", 1)[0].strip("/")
    return project, ref.strip() or None


def project_id(project):
    return urllib.parse.quote(project, safe="")


def api(method, path, params=None, body=None, with_headers=False):
    url, token, insecure = config()
    full = f"{url}/api/v4{path}"
    if params:
        full += "?" + urllib.parse.urlencode(params)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(full, data=data, method=method)
    req.add_header("PRIVATE-TOKEN", token)
    req.add_header("Accept", "application/json")
    req.add_header("User-Agent", "PerritoDeck")
    if data is not None:
        req.add_header("Content-Type", "application/json")
    context = tls_context(insecure) if full.startswith("https://") else None
    try:
        with urllib.request.urlopen(req, timeout=TIMEOUT, context=context) as resp:
            raw = resp.read()
            headers = resp.headers
    except urllib.error.HTTPError as e:
        raise GitLabError(describe_http_error(e)) from None
    except urllib.error.URLError as e:
        raise GitLabError(describe_url_error(url, e.reason)) from None
    except OSError as e:
        raise GitLabError(describe_url_error(url, e)) from None
    try:
        answer = json.loads(raw.decode("utf-8") or "null")
    except ValueError:
        raise GitLabError("GitLab sent an answer that isn't JSON; check the address") from None
    return (answer, headers) if with_headers else answer


def describe_http_error(e):
    message = ""
    try:
        body = json.loads(e.read().decode("utf-8") or "{}")
        message = body.get("message") or body.get("error") or ""
        if isinstance(message, (dict, list)):
            message = json.dumps(message)
    except Exception:
        pass
    if e.code == 401:
        return "GitLab refused the token: check it hasn't expired"
    if e.code == 403:
        return "the token isn't allowed to do that (running pipelines needs the api scope)"
    if e.code == 404:
        return "GitLab can't find that project (check the path, and that the token can see it)"
    return f"GitLab said {e.code}: {message}" if message else f"GitLab said {e.code} {e.reason}"


def describe_url_error(url, reason):
    text = str(reason)
    if isinstance(reason, ssl.SSLCertVerificationError) or "CERTIFICATE_VERIFY_FAILED" in text:
        return f"couldn't check the certificate of {url}; install a trusted one or turn on \"Skip certificate check\""
    return f"couldn't reach GitLab at {url}: {plain_reason(reason)}"


def plain_reason(reason):
    """"[Errno 111] Connection refused" -> "connection refused"."""
    if isinstance(reason, OSError) and reason.strerror:
        text = reason.strerror
    else:
        text = re.sub(r"^\[Errno -?\d+\]\s*", "", str(reason))
    if "timed out" in text:
        return "no answer (timed out)"
    return text[:1].lower() + text[1:] if text[:2] != text[:2].upper() else text


def pipelines(project, ref=None, **filters):
    params = {"per_page": 1, "order_by": "id", "sort": "desc"}
    params.update(filters)
    if ref:
        params["ref"] = ref
    return api("GET", f"/projects/{project_id(project)}/pipelines", params) or []


def latest_pipeline(project, ref=None):
    found = pipelines(project, ref)
    return found[0] if found else None


def refresh_soon():
    timer = threading.Timer(REFRESH_AFTER_ACTION, safe_poll)
    timer.daemon = True
    timer.start()


# --- actions -----------------------------------------------------------------------


def parse_variables(text):
    """KEY=VALUE, one per line (or separated by ';' on a single line)."""
    text = (text or "").strip()
    if not text:
        return []
    lines = text.splitlines() if "\n" in text else text.split(";")
    variables = []
    for line in lines:
        line = line.strip()
        if not line:
            continue
        key, sep, value = line.partition("=")
        key = key.strip()
        if not sep or not key:
            raise GitLabError(f"variables are KEY=VALUE, not {line!r}")
        variables.append({"key": key, "value": value.strip(), "variable_type": "env_var"})
    return variables


@plugin.action("run-pipeline")
def run_pipeline(settings):
    project, ref_in_path = split_target(settings.get("project"))
    ref = (settings.get("ref") or "").strip() or ref_in_path
    if not ref:
        info = api("GET", f"/projects/{project_id(project)}") or {}
        ref = info.get("default_branch")
        if not ref:
            raise GitLabError("the project has no default branch; enter a branch or tag")
    body = {"ref": ref}
    variables = parse_variables(settings.get("variables"))
    if variables:
        body["variables"] = variables
    api("POST", f"/projects/{project_id(project)}/pipeline", body=body)
    refresh_soon()
    return None


@plugin.action("retry-pipeline")
def retry_pipeline(settings):
    project, ref = split_target(settings.get("project"))
    ref = (settings.get("ref") or "").strip() or ref
    failed = pipelines(project, ref, status="failed")
    if not failed:
        return "no failed pipeline to retry"
    api("POST", f"/projects/{project_id(project)}/pipelines/{failed[0]['id']}/retry")
    refresh_soon()
    return None


@plugin.action("cancel-pipelines")
def cancel_pipelines(settings):
    project, ref = split_target(settings.get("project"))
    ref = (settings.get("ref") or "").strip() or ref
    found = {}
    for scope in ("running", "pending"):
        for p in pipelines(project, ref, scope=scope, per_page=20):
            found[p["id"]] = p
    if not found:
        return "no pipeline is running"
    for pipeline_id in sorted(found):
        api("POST", f"/projects/{project_id(project)}/pipelines/{pipeline_id}/cancel")
    refresh_soon()
    return None


@plugin.action("open-pipeline")
def open_pipeline(settings):
    project, ref = split_target(settings.get("project"))
    ref = (settings.get("ref") or "").strip() or ref
    latest = latest_pipeline(project, ref)
    if latest and latest.get("web_url"):
        page = latest["web_url"]
    else:
        page = f"{config()[0]}/{project}/-/pipelines"
    if not webbrowser.open(page):
        raise GitLabError("couldn't open a web browser")
    return None


# --- live values ---------------------------------------------------------------------


def count(path, params):
    found, headers = api("GET", path, dict(params, per_page=1), with_headers=True)
    total = headers.get("X-Total") if headers is not None else None
    if total is not None and str(total).isdigit():
        return str(total)
    # GitLab leaves X-Total out above 10,000 results.
    return "10000+" if found else "0"


def pipeline_values(pipeline):
    if pipeline is None:
        return {"pipeline": "none", "pipeline_ok": "off", "pipeline_running": "off"}
    status = pipeline.get("status") or NOTHING
    return {
        "pipeline": status,
        "pipeline_ok": "on" if status == "success" else "off",
        "pipeline_running": "on" if status in ACTIVE else "off",
    }


def poll():
    try:
        config()
    except GitLabError:
        publish_unknown()
        return
    targets = sorted({arg for v in PIPELINE_VALUES for arg in plugin.watched(v)})
    for arg in targets:
        try:
            project, ref = split_target(arg)
            values = pipeline_values(latest_pipeline(project, ref))
        except GitLabError as e:
            plugin.log(f"{arg}: {e}", "warn")
            values = dict.fromkeys(PIPELINE_VALUES, NOTHING)
        for value_id in PIPELINE_VALUES:
            if arg in plugin.watched(value_id):
                plugin.set_value(value_id, values[value_id], arg=arg)
    for value_id, endpoint in (("mrs", "merge_requests"), ("issues", "issues")):
        for arg in plugin.watched(value_id):
            try:
                project, _ = split_target(arg)
                value = count(f"/projects/{project_id(project)}/{endpoint}", {"state": "opened"})
            except GitLabError as e:
                plugin.log(f"{arg}: {e}", "warn")
                value = NOTHING
            plugin.set_value(value_id, value, arg=arg)


def publish_unknown():
    for value_id in PIPELINE_VALUES + COUNT_VALUES:
        for arg in plugin.watched(value_id):
            plugin.set_value(value_id, NOTHING, arg=arg)


_poll_lock = threading.Lock()


def safe_poll():
    if not _poll_lock.acquire(blocking=False):
        return
    try:
        _last_poll["at"] = time.monotonic()
        poll()
    except Exception as e:
        plugin.log(f"poll failed: {e}", "warn")
    finally:
        _poll_lock.release()


def interval():
    try:
        host = urllib.parse.urlparse(config()[0]).hostname or ""
    except GitLabError:
        return POLL_SECONDS
    return GITLAB_COM_POLL_SECONDS if host == "gitlab.com" or host.endswith(".gitlab.com") else POLL_SECONDS


@plugin.every(POLL_SECONDS)
def tick():
    if not any(plugin.watched(v) for v in PIPELINE_VALUES + COUNT_VALUES):
        return
    if _last_poll["at"] and time.monotonic() - _last_poll["at"] < interval() - 1:
        return
    safe_poll()


@plugin.on_watch
def watch_changed(values):
    threading.Thread(target=safe_poll, daemon=True).start()


if __name__ == "__main__":
    plugin.run()
