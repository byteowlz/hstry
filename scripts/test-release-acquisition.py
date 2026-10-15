#!/usr/bin/env python3
"""Validate a canonical hstry archive and acquire it into an isolated destination."""
import hashlib
import pathlib
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

repo = pathlib.Path(__file__).resolve().parents[1]
version = tomllib.loads((repo / "Cargo.toml").read_text())["workspace"]["package"]["version"]
config = tomllib.loads((repo / "byt.release.toml").read_text())["release"]
triple = "x86_64-unknown-linux-gnu"
root = f'hstry-v{version}-{triple}'
archive = repo / "dist/release" / f"{root}.tar.gz"
expected = {f'{root}/bin/{name}' for name in config["bins"]}
with tarfile.open(archive) as bundle:
    members = bundle.getmembers()
    files = set()
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        assert not path.is_absolute() and ".." not in path.parts
        assert path.parts[0] == root and len(member.name.encode()) <= 4096
        assert member.isdir() or member.isfile(), f"Unsafe member: {member.name}"
        if member.isfile():
            files.add(member.name)
            if member.name in expected:
                assert member.mode & 0o111, f"Non-executable binary: {member.name}"
    assert files == expected | {f"{root}/LICENSE", f"{root}/README.md"}, files
checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
lines = (archive.parent / "checksums.txt").read_text().splitlines()
assert f"{checksum}  {archive.name}" in lines
with tempfile.TemporaryDirectory(prefix="hstry-acquire-") as temporary:
    stage = pathlib.Path(temporary)
    source = stage / "source/hstry/releases/download" / f"v{version}"
    source.mkdir(parents=True)
    shutil.copy(archive, source)
    (source / "checksums.txt").write_text(f"{checksum}  {archive.name}\n")
    manifest = stage / "dependencies.toml"
    manifest.write_text(f'[oqto]\nversion = "0.5.0"\n[byteowlz]\nhstry = "{version}"\n[binary_owners]\nhstry = ["hstry", "hstry-tui", "hstry-mcp"]\n')
    destination = stage / "bin"
    subprocess.run(["oqto-setup", "acquire", "--tools-only", "--manifest", str(manifest), "--base-url", (stage / "source").as_uri(), "--install-bin", str(destination)], check=True)
    for name in config["bins"]:
        subprocess.run([str(destination / name), "--help"], check=True, stdout=subprocess.DEVNULL)
    subprocess.run([str(destination / "hstry"), "--version"], check=True)
print("Archive layout, entrypoint allowlist, checksums and isolated acquisition passed.")
