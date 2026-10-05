#!/usr/bin/python3
"""scan-layers.py: read a docker-archive image (`podman save`) on stdin and
report token-shaped strings in any file of any layer, files that a later
layer deletes included (they are still in the published layer).

Prints one line per hit: the layer, the path and the kind of token, never the
token. Exit 0: nothing found; 1: a hit; 2: the archive could not be read.

Every pattern is looked for everywhere, except that the noisy ones (private
key headers and AWS key IDs) are skipped under usr/: Fedora's packages ship
test keys and byte runs that match them. The atlas-* packages' files under
/usr are checked inside the container by check-image-secrets.sh.
"""

import re
import sys
import tarfile

STRICT = {
    "GitHub token": rb"gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}",
    "GitLab token": rb"glpat-[A-Za-z0-9_-]{20}",
    "npm token": rb"npm_[A-Za-z0-9]{36}",
    "Google API key": rb"AIza[0-9A-Za-z_-]{35}",
    "Slack token": rb"xox[abeprs]-[0-9A-Za-z-]{10,}|xapp-[0-9A-Za-z-]{10,}",
}
NOISY = {
    "AWS key ID": rb"(?:AKIA|ASIA)[0-9A-Z]{16}",
    "private key": rb"-----BEGIN [A-Z ]*PRIVATE KEY-----",
}
CHUNK = 8 << 20
OVERLAP = 256


def compile_all(table):
    return [(kind, re.compile(p)) for kind, p in table.items()]


ALL = compile_all(STRICT) + compile_all(NOISY)
STRICT_ONLY = compile_all(STRICT)


def scan_file(f, patterns):
    """Kinds of token found in the file object f, read in overlapping chunks."""
    found = set()
    tail = b""
    while True:
        block = f.read(CHUNK)
        if not block:
            return found
        data = tail + block
        for kind, rx in patterns:
            if kind not in found and rx.search(data):
                found.add(kind)
        tail = data[-OVERLAP:]


def scan_layer(name, layer, hits):
    for m in layer:
        if not m.isfile():
            continue
        path = m.name.lstrip("./")
        patterns = STRICT_ONLY if path.startswith("usr/") else ALL
        f = layer.extractfile(m)
        if f is None:
            continue
        for kind in sorted(scan_file(f, patterns)):
            # path only: a file name is not the secret
            hits.append(f"{name}: /{path} ({kind})")


def main():
    hits = []
    layers = 0
    try:
        with tarfile.open(fileobj=sys.stdin.buffer, mode="r|") as image:
            for member in image:
                if not member.isfile():
                    continue
                f = image.extractfile(member)
                if member.name.endswith(".tar"):
                    layers += 1
                    with tarfile.open(fileobj=f, mode="r|") as layer:
                        scan_layer(member.name, layer, hits)
                else:
                    # manifest and config JSON
                    for kind in sorted(scan_file(f, ALL)):
                        hits.append(f"{member.name} ({kind})")
    except (tarfile.TarError, OSError, EOFError) as e:
        print(f"cannot read the image archive: {type(e).__name__}", file=sys.stderr)
        return 2
    if layers == 0:
        print("no layers in the image archive", file=sys.stderr)
        return 2
    for h in hits[:20]:
        print(h)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
