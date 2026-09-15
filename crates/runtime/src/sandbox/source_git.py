"""Fixed isolated Git query driver. Input paths and operations are runtime-owned."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ENV = {
    "PATH": "/usr/local/bin:/usr/bin:/bin", "HOME": "/tmp", "LANG": "C.UTF-8",
    "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_SYSTEM": "/dev/null",
    "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_TERMINAL_PROMPT": "0",
    "GIT_OPTIONAL_LOCKS": "0", "GIT_NO_REPLACE_OBJECTS": "1",
    "GIT_NO_LAZY_FETCH": "1", "GIT_ATTR_NOSYSTEM": "1", "GIT_PAGER": "cat",
}
OPERATIONS = {
    "git_diff": ["diff", "--no-ext-diff", "--no-textconv", "--ignore-submodules=all"],
    "git_staged_diff": ["diff", "--cached", "--no-ext-diff", "--no-textconv", "--ignore-submodules=all"],
    "git_log": ["log", "--max-count=20", "--format=oneline", "--no-decorate", "--no-patch"],
    "git_status": ["status", "--porcelain=v1", "--untracked-files=normal", "--ignore-submodules=all"],
}


def main():
    operation = sys.argv[1]
    inputs, worktree = Path(sys.argv[2]), Path(sys.argv[3])
    if operation not in OPERATIONS:
        raise ValueError("unsupported Git source operation")
    # --file and --no-includes prevent source config from selecting any other
    # file. Parsing happens inside the isolated worker, never on the host.
    parsed = subprocess.run(["git", "config", "--file", str(inputs / "configuration"),
                             "--no-includes", "--null", "--list"], env=ENV, capture_output=True)
    if parsed.returncode:
        raise ValueError("invalid repository configuration")
    selected = {}
    for record in parsed.stdout.split(b"\0"):
        if not record:
            continue
        key, _, value = record.partition(b"\n")
        key = key.decode("utf-8", "replace").lower()
        if key == "core.repositoryformatversion" or key.startswith("extensions."):
            if key in selected:
                raise ValueError("ambiguous repository format")
            selected[key] = value.decode("ascii", "strict").strip().lower()
    if selected.get("core.repositoryformatversion", "0") not in ("0", "1"):
        raise ValueError("unsupported repository format")
    if any(k.startswith("extensions.") and k != "extensions.objectformat" for k in selected):
        raise ValueError("unsupported repository extension")
    object_format = selected.get("extensions.objectformat", "sha1")
    if object_format not in ("sha1", "sha256"):
        raise ValueError("unsupported object format")
    git_dir = Path("/tmp/symbi-git-query")
    git_dir.mkdir(mode=0o700)
    for path in (inputs / "metadata").iterdir():
        if path.is_dir():
            (git_dir / path.name).symlink_to(path)
        else:
            # Git validates HEAD as a ref or a regular file. An arbitrary
            # absolute symlink is not a valid HEAD, even with --git-dir.
            shutil.copyfile(path, git_dir / path.name)
    config = "[core]\nrepositoryformatversion = " + ("1" if object_format == "sha256" else "0")
    config += "\nbare = false\nfilemode = true\nsymlinks = true\nfsmonitor = false\nhooksPath = /dev/null\npager = cat\n"
    config += "[protocol]\nallow = never\n"
    if object_format == "sha256":
        config += "[extensions]\nobjectFormat = sha256\n"
    (git_dir / "config").write_text(config)
    os.execvpe("git", ["git", "--no-pager", "--no-optional-locks", "--no-replace-objects",
        "--git-dir=" + str(git_dir), "--work-tree=" + str(worktree), "-c", "safe.directory=" + str(worktree),
        "-c", "core.attributesFile=/dev/null", "-c", "core.excludesFile=/dev/null",
        *OPERATIONS[operation], "--"], ENV)


try:
    main()
except (OSError, ValueError) as error:
    print(json.dumps({"error": str(error)}), file=sys.stderr)
    sys.exit(125)
