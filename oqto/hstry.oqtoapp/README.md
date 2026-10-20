# hstry Oqto App

Find, read and reuse past coding-agent sessions inside Oqto. It is the same
web UI that `hstry ui` serves standalone, running in Oqto's sandboxed frame
with the host's theme.

## Install

The runner needs `hstry` (with your synced history) and `jq` on its `PATH`.

```bash
just oqto-app-install /path/to/workdir   # copies this package to <workdir>/oqto-apps/
```

Then publish and approve it in Oqto. The App requests only `operations` and
`theme`; it never sees a path, the database or the network.

## Agent interface

The App owns no state. History lives in hstry's database; the App is a view.

- **Canonical data**: the hstry database, read through the hstry CLI.
  Agents can use the CLI directly (`hstry list --json`, `hstry show --full --json <id>`,
  `hstry search --json <query>`, `hstry source list --json`) or the same
  operation ids the App uses:

| Operation | CLI behind it | Input |
|---|---|---|
| `hstry.sessions.list` | `hstry list --json` | `{limit?: 1..1500}` |
| `hstry.sessions.show` | `hstry show --full --json` | `{id, offset?, limit?: 1..400}` |
| `hstry.search` | `hstry search --json` | `{query, limit?: 1..500}` |
| `hstry.sources.list` | `hstry source list --json` | `{}` |

- **Output**: the CLI's `{ok, result, error}` envelope. Oqto caps operation
  output at 1 MiB and 10 000 JSON nodes, so `operations/hstry-bridge` bounds it:
  - list-shaped results (sessions, transcript messages, search hits) are
    `"json-rows"`: each row is a JSON-encoded string containing exactly the CLI's
    object (`JSON.parse` / `jq fromjson` it);
  - the session list is compact (title, first prompt trimmed) and fills up to
    the size budget; `truncated: true` says it stopped early;
  - transcripts are paged by message (`page.next_offset`, `page.has_more`);
    long outputs are cut with an explicit
    `[… N more characters not shown here …]` note, never silently.
- **Mutations**: none. The App is read-only; continuing a session copies the
  `hstry resume <id> --agent <agent>` command for the user to run.
- **Errors**: `{"error": {"message": "..."}}` on stdout with a non-zero exit.

## Build and verify

The bundle is generated, never edited by hand:

```bash
just ui-build     # web/ -> bundle/index.html (+ crates/hstry-api/ui/index.html)
just ui-test      # web unit tests + this App's data path through the SDK test host
```

`web/test/oqto.test.ts` drives the real bridge and hstry binary through the App
SDK's test host and asserts every operation stays inside Oqto's limits.
Editing anything in this package changes its digest and requires republish.
