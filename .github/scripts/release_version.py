"""Choose release tags and stamp both apps in the build checkout (no commits)."""

import argparse
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[2]
VERSION = re.compile(r"v?(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")


def parse_version(value):
    match = VERSION.fullmatch(value)
    if not match:
        raise ValueError(f"Expected a stable version such as v0.1.5, got {value!r}")
    return tuple(map(int, match.groups()))


def choose_version(tags, minimum, current_tags=(), requested=""):
    if requested:
        version = parse_version(requested)
    else:
        # Rerunning the same commit repairs its release instead of consuming a patch.
        floor = parse_version(minimum)
        existing = [parse_version(t) for t in current_tags if t.startswith("v") and VERSION.fullmatch(t) and parse_version(t) >= floor]
        if existing:
            version = max(existing)
        else:
            stable = [parse_version(t) for t in tags if t.startswith("v") and VERSION.fullmatch(t)]
            latest = max(stable, default=(0, 0, 0))
            version = max(floor, (*latest[:2], latest[2] + 1))
    return ".".join(map(str, version))


def version_code(version):
    major, minor, patch = parse_version(version)
    code = major * 1_000_000 + minor * 1_000 + patch
    if minor >= 1000 or patch >= 1000 or not 1 <= code <= 2_100_000_000:
        raise ValueError("Version exceeds the Android versionCode range")
    return code


def stamp(version, root=ROOT):
    code = version_code(version)
    changes = {
        "linux/Cargo.toml": [(r'(?m)^version = "[^"]+"$', f'version = "{version}"')],
        "linux/Cargo.lock": [(r'(?m)(^name = "airbridge"\nversion = ")[^"]+("$)', rf'\g<1>{version}\2')],
        "android/app/build.gradle.kts": [
            (r'(?m)^(\s*versionName = ")[^"]+("$)', rf'\g<1>{version}\2'),
            (r'(?m)^(\s*versionCode = )\d+$', rf'\g<1>{code}'),
        ],
    }
    for name, replacements in changes.items():
        path = root / name
        text = path.read_text()
        for pattern, replacement in replacements:
            text, count = re.subn(pattern, replacement, text)
            if count != 1:
                raise ValueError(f"Expected one version field in {name}, found {count}")
        path.write_text(text)


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("select", "stamp"))
    parser.add_argument("version", nargs="?", default="")
    parser.add_argument("--automatic", action="store_true")
    args = parser.parse_args()
    if args.command == "stamp":
        stamp(".".join(map(str, parse_version(args.version))))
        return
    tags = git("tag", "--list").splitlines()
    current = git("tag", "--points-at", "HEAD").splitlines() if args.automatic else []
    minimum = tomllib.loads((ROOT / "linux/Cargo.toml").read_text())["package"]["version"]
    version = choose_version(tags, minimum, current, args.version)
    version_code(version)
    tag = f"v{version}"
    sha = git("rev-parse", "HEAD")
    if tag in tags and git("rev-parse", f"refs/tags/{tag}^{{commit}}") != sha:
        raise ValueError(f"{tag} already belongs to another commit; choose a new version")
    print(f"version={version}\ntag={tag}\nsha={sha}")


if __name__ == "__main__":
    main()
