# /// script
# requires-python = ">=3.10"
# dependencies = ["huggingface_hub==1.32.0"]
#
# [tool.uv]
# exclude-newer = "2026-09-20T00:00:00Z"
# ///
"""Publish an OnRecord data pack to Hugging Face as Writerslogic/onrecord-<pack>.

Usage:
  uv run --locked --script scripts/publish_pack.py <pack-dir>
      a whole curated pack, as built
  uv run --locked --script scripts/publish_pack.py --incremental <work-dir>
      one snapshot run: its manifest, card and the files in new-files.txt
  uv run --locked --script scripts/publish_pack.py --ensure-repo <pack>
      create the public repository if absent

Every dependency is pinned with its hashes in publish_pack.py.lock (`uv lock --script
scripts/publish_pack.py` after changing the metadata above). The token is HF_TOKEN from the
environment, else from ~/.env. Publish a curated pack only after the app's verifier has passed on it
(`ONRECORD_PACK_DIR=<pack-dir> cargo test --lib a_built_pack_verifies_against_the_pinned_key -- --ignored`);
a snapshot run verifies its own manifest and new files before it exits. The dataset card is
generated from the signed manifest; the manifest itself is uploaded unchanged. Nothing published is
ever deleted or replaced: an incremental upload refuses a manifest that drops a published file or
check, that changes any field of a published file's entry, or that lists a new file at a published
path.
"""

import json
import os
import sys
from collections import defaultdict
from pathlib import Path, PurePosixPath

from huggingface_hub import CommitOperationAdd, HfApi, hf_hub_download
from huggingface_hub.utils import EntryNotFoundError, RepositoryNotFoundError

ORG = "Writerslogic"
SNAPSHOT_PACK = "san-diego-snapshots"
# Must equal PACK_PUBLIC_KEYS in src-tauri/src/packs/mod.rs: (public key, signs the snapshot pack).
PUBLIC_KEYS = {
    "pack-2026-1": ("b42202850cd9ecda4ab2b96548891d3a2a8275f74dde76bba2153826089ca8ff", False),
    "pack-ci-2026-1": ("3049662bc87c61409d8b8a515d60adf1420a32504bd4f6215ac0c121550a318f", True),
}
TITLES = {
    "san-diego-money": "San Diego money: budgets, actuals, capital programme and City property",
    "san-diego-reports": "San Diego reports: Get It Done, police calls for service and Fire-Rescue incidents",
    "san-diego-enforcement": "San Diego enforcement: police stops, offences, parking citations, collisions and hate crimes",
    "san-diego-capacity": "San Diego capacity: development approvals",
    SNAPSHOT_PACK: "San Diego snapshots: daily history of City files overwritten in place",
}


def token() -> str:
    if os.environ.get("HF_TOKEN"):
        return os.environ["HF_TOKEN"]
    env = Path("~/.env").expanduser()
    if env.is_file():
        for line in env.read_text().splitlines():
            if line.startswith("HF_TOKEN="):
                return line.split("=", 1)[1].strip().strip("\"'")
    sys.exit("HF_TOKEN is neither in the environment nor in ~/.env")


def repository(pack: str) -> str:
    return f"{ORG}/onrecord-{pack}"


def key_name(signed: dict) -> str:
    """The pinned name of the manifest's key, refused unless the app accepts that key on its pack."""
    public = signed["signature"].get("publicKeyHex", "").lower()
    pack = signed["manifest"]["pack"]
    for kid, (hex_key, signs_snapshots) in PUBLIC_KEYS.items():
        if hex_key == public:
            if signs_snapshots != (pack == SNAPSHOT_PACK):
                sys.exit(f"the manifest of {pack} is signed by {kid}, a key that does not sign it")
            if signed["manifest"].get("keyId", kid) != kid:
                sys.exit(f"the manifest names {signed['manifest']['keyId']} but is signed by {kid}")
            return kid
    sys.exit("the manifest is not signed with a pinned publisher key")


def common_sections(signed: dict, kid: str) -> str:
    manifest = signed["manifest"]
    licence = manifest["licence"]
    return f"""## Licence

Published by the City of San Diego under the [{licence['id']}]({licence['url']}).
The City's disclaimer, which the licence terms require: {licence['disclaimer']}

## Conformed columns (Parquet)

- `record_id`: the publisher's key, numbered `key:n` when the file repeats it, else `row:<hash>:<n>`;
  `publisher_record_id` keeps the key as published.
- `source_id`, `source_url`, `retrieved_at`, `source_sha256`, `source_row`, `pack_version`.
- `occurred_at` (as recorded, no time zone: the City publishes Pacific wall-clock time),
  `occurred_on`, `calendar_year`, `fiscal_year` (City fiscal year, named by the June it ends in),
  `time_basis` (the publisher column the time came from). Absent or unreadable values are NULL.

## Verifying

`manifest.json` lists every file with its source URL, retrieval time, SHA-256, size and row count,
and is signed with Ed25519 by key `{kid}` (public key `{PUBLIC_KEYS[kid][0]}`, key id
`{signed['signature']['signerKeyId']}`, the SHA-256 of the public key). OnRecord Studio pins two
publisher keys and accepts each only on its own packs: `pack-2026-1`
(`{PUBLIC_KEYS['pack-2026-1'][0]}`), held offline by the owner, signs the curated releases and
never the snapshot pack; `pack-ci-2026-1` (`{PUBLIC_KEYS['pack-ci-2026-1'][0]}`), held by the
scheduled snapshot job, signs only `{SNAPSHOT_PACK}`. The app refuses a pack whose signature, key,
key-to-pack binding or any file hash does not verify. Any original can also be re-checked against its publisher URL.

A parking citation, a call for service or a permit record states what the City recorded, not a
finding about any person or a legal conclusion.
"""


def curated_card(signed: dict, kid: str) -> str:
    manifest = signed["manifest"]
    pack = manifest["pack"]
    tables = defaultdict(lambda: {"files": 0, "rows": 0, "years": set(), "sources": set(), "partial": False})
    for file in manifest["files"]:
        if file["format"] != "ORIGINAL":
            continue
        table = tables[file["table"]]
        table["files"] += 1
        table["rows"] += file["rows"]
        table["partial"] |= file["partial"]
        if file["year"] is not None:
            table["years"].add(file["year"])
        table["sources"].add(file["sourceUrl"].rsplit("/", 1)[0] + "/")
    rows = []
    for name, table in sorted(tables.items()):
        years = f"{min(table['years'])}–{max(table['years'])}" if table["years"] else "one file"
        rows.append(
            f"| `{name}` | {table['files']} | {table['rows']:,} | {years} | {'yes' if table['partial'] else 'no'} | "
            + ", ".join(sorted(table["sources"]))
            + " |"
        )
    configs = "\n".join(
        f"- config_name: {name}\n  data_files: parquet/{name}/*.parquet" for name in sorted(tables)
    )
    return f"""---
license: pddl
pretty_name: "{TITLES.get(pack, pack)}"
language:
- en
tags:
- public-records
- open-government
- san-diego
- accountability
configs:
{configs}
---

# {TITLES.get(pack, pack)}

City of San Diego open data, retrieved by the OnRecord Studio pack builder on {manifest['builtAt'][:10]}
and published as pack `{pack}` version `{manifest['packVersion']}` (conformed schema {manifest['schemaVersion']}).

Every file is the publisher's original, byte for byte, beside a Parquet copy that keeps every
publisher column and adds standard identity, provenance and time columns. Rows are the builder's
retrieval as of the build date; a file marked partial covers a period not yet over, so the
publisher may still add or correct rows.

| Table | Files | Rows | Years | Partial | Source |
|---|---|---|---|---|---|
{chr(10).join(rows)}

{common_sections(signed, kid)}"""


def snapshot_card(signed: dict, kid: str) -> str:
    manifest = signed["manifest"]
    pack = manifest["pack"]
    tables = defaultdict(lambda: {"snapshots": [], "checks": [], "changes": 0, "source": ""})
    for file in manifest["files"]:
        table = tables[file["table"]]
        if file["format"] == "ORIGINAL":
            table["snapshots"].append(file)
            table["source"] = file["sourceUrl"]
        elif file["format"] == "CHANGES":
            table["changes"] += 1
    for check in manifest.get("checks", []):
        tables[check["table"]]["checks"].append(check["checkedAt"])
    rows = []
    for name, table in sorted(tables.items()):
        latest = max(table["snapshots"], key=lambda file: file["retrievedAt"])
        last_check = max(table["checks"])[:10] if table["checks"] else "none"
        rows.append(
            f"| `{name}` | {len(table['snapshots'])} | {latest['retrievedAt'][:10]} | {latest['rows']:,} | "
            f"{len(table['checks'])} | {last_check} | {table['changes']} | {table['source']} |"
        )
    configs = "\n".join(
        f"- config_name: {name}\n  data_files: parquet/{name}/*/*.parquet" for name in sorted(tables)
    )
    return f"""---
license: pddl
pretty_name: "{TITLES[pack]}"
language:
- en
tags:
- public-records
- open-government
- san-diego
- accountability
- snapshots
configs:
{configs}
---

# {TITLES[pack]}

The City of San Diego overwrites many of its open-data files in place: open Get It Done reports,
City properties, parcels and leases, budget and actuals files, lookup lists, police stop records,
collisions and hate crimes. Once overwritten, the earlier file exists nowhere. A scheduled job
retrieves each of these files every day at 13:23 UTC and keeps what it received. This is pack
`{pack}` version `{manifest['packVersion']}`, last extended {manifest['builtAt'][:10]}.

- **Snapshot.** When a file's bytes differ from its latest snapshot, the publisher's original is
  kept byte for byte at `originals/<table>/<stamp>/<file>`, with a Parquet copy at
  `parquet/<table>/<stamp>/<stem>.parquet`; `<stamp>` is the UTC retrieval time
  (`YYYY-MM-DDTHHMMSSZ`).
- **Check.** When the bytes equal the latest snapshot, the manifest records a check: the retrieval
  time, size, SHA-256, the publisher's `Last-Modified` and `ETag`, and the snapshot it matched. A
  check says the file was unchanged when retrieved; it says nothing about the file between
  retrievals, so a change made and undone between two checks is not seen.
- **Change record.** Each new snapshot after the first has `changes/<table>/<stamp>.parquet`,
  comparing it with the snapshot before, on the publisher's own cell text: a row per record added
  or removed, and per field changed (`change`, `publisher_record_id`, `key_occurrence`, `field`,
  `before`, `after`, `note`). A table with a publisher key compares records by key and the key's
  repeat in file order; other rows compare by their cells in the columns both snapshots have,
  counting duplicates. When more than half
  the records changed, changed records are listed whole as removed and added, and `note` says so.
  A column present on one side only is one `column_added` or `column_removed` row.
- **Immutable.** A published file is never replaced or deleted; each run only adds files and
  checks, and signs the cumulative manifest again.

| Table | Snapshots | Latest snapshot | Its rows | Unchanged checks | Last check | Change records | Source |
|---|---|---|---|---|---|---|---|
{chr(10).join(rows)}

{common_sections(signed, kid)}"""


def card(signed: dict, kid: str) -> str:
    if signed["manifest"]["pack"] == SNAPSHOT_PACK:
        return snapshot_card(signed, kid)
    return curated_card(signed, kid)


def read_signed(directory: Path) -> tuple[dict, str]:
    signed = json.loads((directory / "manifest.json").read_text())
    return signed, key_name(signed)


def remote_manifest(api: HfApi, repo: str, revision: str) -> dict | None:
    try:
        path = hf_hub_download(
            repo, "manifest.json", repo_type="dataset", revision=revision, token=api.token
        )
    except EntryNotFoundError:
        return None
    return json.loads(Path(path).read_text())["manifest"]


def checked_new_files(directory: Path, manifest: dict) -> list[str]:
    listed = {file["path"] for file in manifest["files"]}
    paths = []
    for line in (directory / "new-files.txt").read_text().splitlines():
        part = PurePosixPath(line)
        if (
            not line
            or part.is_absolute()
            or "\\" in line
            or any(segment in ("", ".", "..") for segment in line.split("/"))
        ):
            sys.exit(f"new-files.txt lists a path outside the pack: {line!r}")
        if line not in listed:
            sys.exit(f"new-files.txt lists {line}, which the manifest does not")
        if not (directory / line).is_file():
            sys.exit(f"{line} is missing from {directory}")
        paths.append(line)
    return paths


def replaces_published(published: dict, manifest: dict, new_files: list[str]) -> str | None:
    """Why `manifest` with `new_files` would drop or replace what `published` holds, if it would."""
    current = {file["path"]: file for file in manifest["files"]}
    dropped = [file["path"] for file in published["files"] if current.get(file["path"]) != file]
    if dropped:
        return f"the new manifest drops or changes published files: {dropped[:5]}"
    kept = published.get("checks", [])
    if manifest.get("checks", [])[: len(kept)] != kept:
        return "the new manifest does not keep every published check"
    clashing = {file["path"] for file in published["files"]} & set(new_files)
    if clashing:
        return f"new files would replace published ones: {sorted(clashing)[:5]}"
    return None


def publish_incremental(directory: Path) -> None:
    signed, kid = read_signed(directory)
    manifest = signed["manifest"]
    pack = manifest["pack"]
    new_files = checked_new_files(directory, manifest)
    repo = repository(pack)
    api = HfApi(token=token())
    head = api.dataset_info(repo).sha
    published = remote_manifest(api, repo, head)
    if published is not None:
        refusal = replaces_published(published, manifest, new_files)
        if refusal:
            sys.exit(refusal)
    (directory / "README.md").write_text(card(signed, kid))
    operations = [
        CommitOperationAdd(path_in_repo=path, path_or_fileobj=str(directory / path))
        for path in [*new_files, "manifest.json", "README.md"]
    ]
    # One commit, on the commit the published manifest was read from: the app fetches a manifest's
    # files from the commit it came from, and a concurrent publish fails rather than interleaves.
    commit = api.create_commit(
        repo_id=repo,
        repo_type="dataset",
        operations=operations,
        parent_commit=head,
        commit_message=f"Snapshot {pack} {manifest['packVersion']}: {len(new_files)} new files, "
        f"{len(manifest.get('checks', []))} checks",
    )
    print(repo, commit.oid)


def publish_full(directory: Path) -> None:
    signed, kid = read_signed(directory)
    pack = signed["manifest"]["pack"]
    if pack == SNAPSHOT_PACK:
        sys.exit(f"{SNAPSHOT_PACK} is published only with --incremental, so no published file is replaced")
    (directory / "README.md").write_text(card(signed, kid))
    repo = repository(pack)
    api = HfApi(token=token())
    api.create_repo(repo, repo_type="dataset", exist_ok=True, private=False)
    commit = api.upload_folder(
        repo_id=repo,
        repo_type="dataset",
        folder_path=directory,
        allow_patterns=["manifest.json", "README.md", "originals/**", "parquet/**"],
        commit_message=f"Pack {pack} {signed['manifest']['packVersion']}",
    )
    print(repo, commit.oid if hasattr(commit, "oid") else commit)


def ensure_repository(pack: str) -> None:
    if pack not in TITLES:
        sys.exit(f"'{pack}' is not a pack OnRecord publishes")
    api = HfApi(token=token())
    try:
        api.dataset_info(repository(pack))
    except RepositoryNotFoundError:
        api.create_repo(repository(pack), repo_type="dataset", private=False)
        print("created", repository(pack))


def main() -> None:
    match sys.argv[1:]:
        case ["--incremental", directory]:
            publish_incremental(Path(directory).resolve())
        case ["--ensure-repo", pack]:
            ensure_repository(pack)
        case [directory] if not directory.startswith("--"):
            publish_full(Path(directory).resolve())
        case _:
            sys.exit(__doc__)


if __name__ == "__main__":
    main()
