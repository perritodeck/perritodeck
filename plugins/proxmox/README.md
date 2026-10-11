# Proxmox VE

Start, shut down, restart and snapshot Proxmox VE virtual machines and containers from
PerritoDeck, and show their status, CPU, memory and uptime on keys. Plugin id:
`dev.perritodeck.proxmox`.

It uses the Proxmox VE REST API (`/api2/json`) with an API token. Keys only need the
guest's id (vmid, e.g. `101`): the plugin looks up which node it's on and whether it's
a VM (qemu) or a container (lxc) in `/cluster/resources`. That one call is also all a
poll makes: every 5 seconds while a key shows a VM or node value, every 30 seconds
otherwise (for the running count). Python 3.8+ standard library only.

## Setup

1. In the Proxmox web interface, make a role with just what the plugin needs
   (**Datacenter → Permissions → Roles → Create**), or on a node's shell:

   ```sh
   pveum role add PerritoDeck -privs "VM.Audit,VM.PowerMgmt,VM.Snapshot,Sys.Audit"
   ```

   | Privilege | Needed for |
   |---|---|
   | `VM.Audit` | Seeing guests at all (status, CPU, memory, names). Without it a guest looks missing. |
   | `VM.PowerMgmt` | Start, shut down, stop, restart, pause and resume |
   | `VM.Snapshot` | Taking snapshots (leave it out if you don't want that) |
   | `Sys.Audit` | Node CPU and memory (`node_cpu`, `node_mem`); without it those show `–` |

2. Make an API token (**Datacenter → Permissions → API Tokens → Add**), e.g. user
   `root@pam`, token id `perritodeck`. Copy the secret: Proxmox shows it only once.

   Leave **Privilege Separation** ticked (the safer default). A separated token has
   *no* rights of its own: give the token (not just the user) the role on `/`, or on
   `/vms/101` etc. to limit it to some guests, plus `/nodes` for node values:

   ```sh
   pveum user token add root@pam perritodeck --privsep 1
   pveum acl modify / --tokens 'root@pam!perritodeck' --roles PerritoDeck
   ```

   A separated token can never do more than its user, so the user needs these rights
   too (`root@pam` has them all). Unticking Privilege Separation gives the token all of
   the user's rights instead.

3. In PerritoDeck, open **Plugins → Proxmox VE** and fill in:
   - **Proxmox address**: `https://pve.local:8006` (any node of the cluster).
   - **API token id**: `root@pam!perritodeck` (user@realm!token name).
   - **API token secret**: the secret from step 2.
   - **Skip certificate check**: tick it if Proxmox still uses the self-signed
     certificate it was installed with. Better: give Proxmox a trusted certificate
     (e.g. with its built-in ACME support) and leave this off.

## Actions

Every action has the field **VM or container id** (e.g. `101`).

| Action | What it does (API call) |
|---|---|
| Start | `POST /nodes/{node}/{qemu\|lxc}/{vmid}/status/start` |
| Shut down | `…/status/shutdown`: asks the guest OS to shut down cleanly (needs the QEMU guest agent or ACPI in VMs) |
| Stop (pull the plug) | `…/status/stop` |
| Restart | `…/status/reboot` |
| Pause a VM | `…/qemu/{vmid}/status/suspend` (VMs only) |
| Resume a paused VM | `…/qemu/{vmid}/status/resume` (VMs only) |
| Start or shut down | Start when stopped, otherwise shut down cleanly. Pair it with the value `running` so the key lights up while the guest runs. |
| Take a snapshot | `POST …/snapshot` with `snapname` (field **Snapshot name**, optional: `deck_20261011_201503` when empty) and no RAM state |

Starting a guest that is already running (or stopping one that's stopped) flashes a ⚠
warning and does nothing. Proxmox runs these as tasks; the key answers as soon as the
task is queued, and the values update a couple of seconds later.

## Values

| Value | Example title | Shows |
|---|---|---|
| `status:<vmid>` | `{plugin:dev.perritodeck.proxmox:status:101}` | `running`, `stopped` or `paused` |
| `running:<vmid>` | `{plugin:dev.perritodeck.proxmox:running:101}` | `on` / `off`, for a toggle to follow |
| `cpu:<vmid>` | `CPU {plugin:dev.perritodeck.proxmox:cpu:101}%` | CPU use, 0–100 (of the guest's cores) |
| `mem:<vmid>` | `RAM {plugin:dev.perritodeck.proxmox:mem:101}%` | Memory use, 0–100 |
| `uptime:<vmid>` | `{plugin:dev.perritodeck.proxmox:uptime:101}` | e.g. `3d 4h`, `5h 12m`, `12m`; `–` when stopped |
| `name:<vmid>` | `{plugin:dev.perritodeck.proxmox:name:101}` | The guest's name |
| `node_cpu:<node>` | `{plugin:dev.perritodeck.proxmox:node_cpu:pve}%` | Node CPU use, 0–100 |
| `node_mem:<node>` | `{plugin:dev.perritodeck.proxmox:node_mem:pve}%` | Node memory use, 0–100 |
| `node_status:<node>` | `{plugin:dev.perritodeck.proxmox:node_status:pve}` | `online` / `offline` |
| `guests_running` | `{plugin:dev.perritodeck.proxmox:guests_running} running` | How many VMs and containers run (templates don't count) |

Values show `–` when Proxmox can't be reached or the guest/node isn't found.
`/cluster/resources` reports a paused VM as running, so for a running VM whose `status`
a key shows, the plugin also asks `…/qemu/{vmid}/status/current` for its `qmpstatus`.

## Tested and untested

Tested with unit tests against a fake server that follows the documented API shapes
(paths, `PVEAPIToken` header, form-encoded POSTs, `{"data": …}` answers, errors in the
HTTP reason phrase). Endpoints, fields and privilege checks were checked against the
Proxmox VE API schema and source (`pve-manager` `PVE/API2/Cluster.pm`,
`qemu-server` `PVE/API2/Qemu.pm`).

Not yet tried against a real Proxmox VE server: the exact error texts Proxmox returns,
`qmpstatus` for paused VMs on every PVE version, and the snapshot name rules on older
versions.
