# Remote sync

Each remote host keeps a normal hstry database. Pulling merges the remote database into the local database; pushing first fetches the hub database, merges local data into that copy, validates it, and only then uploads the merged database.

## Configure a remote

```toml
[[remotes]]
name = "hub"
host = "user@hub" # Prefer an alias from ~/.ssh/config
database_path = "~/.local/share/hstry/hstry.db"
enabled = true

[sync]
mode = "satellite"
device_id = "work-laptop" # Recommended stable, unique label
hub_remote = "hub"
```

Run:

```sh
hstry remote test hub
hstry remote sync --remote hub --direction pull
hstry remote sync --remote hub --direction push
```

If `sync.device_id` is omitted, hstry generates a UUID-backed ID once and stores it under the platform state directory (`$XDG_STATE_HOME/hstry/device-id` on XDG systems). Pushed source IDs are namespaced with this ID so two satellites cannot overwrite one another.

## Safety and concurrency

- Remote paths support a leading `~` and simple `$VAR`/`${VAR}` expansion. Other shell characters are treated literally.
- A fetched hub must pass SQLite integrity and hstry-schema checks before hstry modifies or uploads it.
- Pushes are read-modify-write operations. Do not push from multiple satellites concurrently; serialize pushes through scheduling or an external lock.
- Keep SSH host keys and authentication policy in `~/.ssh/config`.

Use `--direction pull` when a hub is authoritative. Use bidirectional sync only when both sides are expected to contribute history.
