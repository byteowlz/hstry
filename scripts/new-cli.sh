#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: new-cli.sh <name> [--path DIR]

Create a new workspace project by cloning the current template into DIR (defaults to <name>).
Renames all crates from rust-* to <name>-* pattern.

Options:
  -h, --help      Show this message
      --path DIR  Destination directory for the new project
USAGE
}

die() {
  echo "new-cli.sh: $*" >&2
  exit 1
}

NAME=""
DEST=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      usage
      exit 0
      ;;
    --path)
      shift
      [[ $# -gt 0 ]] || die "--path requires an argument"
      DEST="$1"
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      if [[ -z "$NAME" ]]; then
        NAME="$1"
      else
        die "unexpected argument: $1"
      fi
      ;;
  esac
  shift
done

[[ -n "$NAME" ]] || die "project name is required"

if [[ ! "$NAME" =~ ^[a-zA-Z][a-zA-Z0-9_-]*$ ]]; then
  die "project name must start with a letter and contain only letters, numbers, '_' or '-'"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMPLATE_ROOT="$(dirname "$SCRIPT_DIR")"

if [[ -z "$DEST" ]]; then
  DEST="$(dirname "$TEMPLATE_ROOT")/$NAME"
fi

case "$DEST" in
  /*) ;; # absolute path
  *) DEST="$PWD/$DEST" ;;
esac

if [[ -e "$DEST" ]]; then
  die "destination already exists: $DEST"
fi

mkdir -p "$(dirname "$DEST")"

python3 - "$TEMPLATE_ROOT" "$DEST" <<'PY'
import pathlib
import shutil
import sys

root = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2])

def ignore(directory, contents):
    ignored = {'.git', 'target', '.DS_Store'}
    return ignored.intersection(contents)

shutil.copytree(root, dest, ignore=ignore)

for path in dest.rglob('new-cli.sh'):
    if path.is_file():
        path.chmod(path.stat().st_mode | 0o111)
PY

python3 - "$NAME" "$DEST" <<'PY'
import pathlib
import re
import sys

name = sys.argv[1]
dest = pathlib.Path(sys.argv[2])

# Replacement patterns
old_workspace = "rust-workspace"
old_prefix = "rust-"
new_prefix = f"{name}-"
old_env_prefix = "RUST_WORKSPACE"
new_env_prefix = name.upper().replace("-", "_")

def replace_content(path: pathlib.Path):
    """Replace all occurrences in file content."""
    text = path.read_text()
    # Replace workspace name
    text = text.replace(old_workspace, name)
    # Replace crate prefixes (hstry-core -> name-core, etc.)
    text = text.replace(old_prefix, new_prefix)
    # Replace environment variable prefix
    text = text.replace(old_env_prefix, new_env_prefix)
    path.write_text(text)

def rename_directories(base: pathlib.Path):
    """Rename crate directories from rust-* to name-*."""
    crates_dir = base / "crates"
    if not crates_dir.exists():
        return

    for crate_dir in sorted(crates_dir.iterdir(), reverse=True):
        if crate_dir.is_dir() and crate_dir.name.startswith(old_prefix):
            new_name = new_prefix + crate_dir.name[len(old_prefix):]
            new_path = crate_dir.parent / new_name
            crate_dir.rename(new_path)

# Files to update
files_to_update = [
    dest / "Cargo.toml",
    dest / "Cargo.lock",
    dest / "README.md",
    dest / "AGENTS.md",
    dest / "TUI.md",
    dest / "examples" / "config.toml",
]

# Update crate files
for crate_toml in dest.glob("crates/*/Cargo.toml"):
    files_to_update.append(crate_toml)

for main_rs in dest.glob("crates/*/src/main.rs"):
    files_to_update.append(main_rs)

for lib_rs in dest.glob("crates/*/src/lib.rs"):
    files_to_update.append(lib_rs)

# Process file content replacements
for file in files_to_update:
    if file.exists():
        replace_content(file)

# Rename crate directories
rename_directories(dest)
PY

echo "Created workspace project at $DEST"
echo "Crates renamed to: ${NAME}-core, ${NAME}-cli, ${NAME}-tui, ${NAME}-mcp, ${NAME}-api"
