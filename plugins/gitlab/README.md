# GitLab

Run, retry and cancel GitLab CI/CD pipelines from PerritoDeck, open the latest one in
the browser, and show pipeline status and open merge request and issue counts on keys.
Works with gitlab.com and self-managed GitLab. Plugin id: `dev.perritodeck.gitlab`.

It uses the GitLab REST API v4 with an access token (`PRIVATE-TOKEN` header). Values
are polled every 30 seconds while a key shows them (every 60 seconds on gitlab.com).
Python 3.8+ standard library only.

## Setup

1. Make an access token. Any of these works:
   - **Personal access token**: avatar → **Edit profile → Access tokens → Add new
     token**.
   - **Project access token** (limited to one project): the project's **Settings →
     Access tokens**. Give it at least the *Developer* role to run pipelines.
   - **Group access token**: the group's **Settings → Access tokens**.

   Scopes: **`read_api`** is enough to show values and open pipelines. Running,
   retrying and cancelling pipelines needs **`api`** (which includes `read_api`).
   Set an expiry date you'll remember: an expired token shows "GitLab refused the
   token".
2. In PerritoDeck, open **Plugins → GitLab** and fill in:
   - **GitLab address**: `https://gitlab.com` (the default) or your own, e.g.
     `https://git.home.lan`.
   - **Access token**: the token, e.g. `glpat-…`.
   - **Skip certificate check**: only for a self-managed GitLab with a self-signed
     certificate.

## Projects and branches

Projects are named by their path, as in the address bar: `my-group/my-project` (or
`group/subgroup/project`). For one branch or tag, add `@branch`:
`my-group/my-project@main`. Without a branch, pipeline values follow the newest pipeline
on any branch.

## Actions

| Action | Fields | What it does (API call) |
|---|---|---|
| Run a pipeline | Project, branch or tag (optional: the default branch), variables (optional) | `POST /projects/:id/pipeline` with `ref` and `variables` |
| Retry the last failed pipeline | Project, branch (optional) | Finds the newest `failed` pipeline, then `POST /projects/:id/pipelines/:id/retry` |
| Cancel running pipelines | Project, branch (optional) | Cancels every `running` and `pending` pipeline: `POST /projects/:id/pipelines/:id/cancel` |
| Open the latest pipeline | Project, branch (optional) | Opens its page (or the project's pipeline list) in the browser on the computer running the host |

Variables are `KEY=VALUE`, separated by `;` (or one per line), e.g.
`DEPLOY=yes; ENV=staging`. They're sent as `env_var` variables. Retry and cancel warn
(⚠) when there's nothing to retry or cancel.

## Values

| Value | Example title | Shows |
|---|---|---|
| `pipeline:<project>` | `{plugin:dev.perritodeck.gitlab:pipeline:my-group/my-project@main}` | The newest pipeline's status: `success`, `failed`, `running`, `pending`, `canceled`, `skipped`, `manual`, `created`…; `none` if there's no pipeline |
| `pipeline_ok:<project>` | `{plugin:dev.perritodeck.gitlab:pipeline_ok:my-group/my-project}` | `on` when the newest pipeline passed, else `off` |
| `pipeline_running:<project>` | `{plugin:dev.perritodeck.gitlab:pipeline_running:my-group/my-project}` | `on` while the newest pipeline is running or waiting |
| `mrs:<project>` | `{plugin:dev.perritodeck.gitlab:mrs:my-group/my-project} MRs` | Open merge requests |
| `issues:<project>` | `{plugin:dev.perritodeck.gitlab:issues:my-group/my-project} issues` | Open issues |

The three pipeline values share one API call per project/branch. The counts come from
GitLab's `X-Total` header; GitLab leaves it out above 10,000, so huge projects show
`10000+`. Values show `–` when GitLab can't be reached or the project isn't found.

## Tested and untested

Tested with unit tests against a fake server following the documented API
(`doc/api/pipelines.md` and the REST pagination docs: URL-encoded project paths,
`per_page`/`order_by`/`sort`/`ref`/`status`/`scope` filters, the `variables` array,
`X-Total`). Not yet tried against a real GitLab instance, and the browser opening isn't
tested on a real desktop.
