#!/usr/bin/python3
"""scan-layers.py [--allow-sha256 HEX]...: read a docker-archive image
(`podman save`) on stdin and report token-shaped strings in any file of any
layer, files that a later layer deletes included (they are still in the
published layer).

Prints one line per hit: the layer, the path (escaped, cut short) and the
kind of token, never the token. Exit 0: nothing found; 1: a hit; 2: the
archive could not be read or is not laid out as expected, or anything else
went wrong.

Every pattern is looked for in every file. Two are stricter than a plain
header or prefix match, because Fedora's programs carry the bare text of key
headers and long capital runs: a private key needs its encoded body after
the header (across real or escaped line breaks, indented or not), and an AWS
key ID must stand alone. ALLOWED names the few Fedora files that ship a
real, public test key; a hit there passes only when the file's sha256 is one
given with --allow-sha256 (check-image-secrets.sh takes them from the
image's RPM database), so a key added to such a file still fails.

The raw byte stream of the archive is what is scanned, once, so data a tar
reader skips (after an end marker, in padding) cannot hide a token. Each match
is mapped back to the file whose data holds it; one outside every file is
reported as "elsewhere". Every member of the archive must be a layer, image
metadata or a link, and every layer the manifest lists must have been read.
"""

import bisect
import hashlib
import json
import posixpath
import re
import sys
import tarfile

# A line break, real or escaped once or more (JSON, JSON in JSON, a quoted
# string), with indentation; bounded, so a match stays far below OVERLAP.
BREAK = rb"(?:\s|\\+[nr]|\\+u000[aAdD])"
B0 = BREAK + rb"{0,64}"
B1 = BREAK + rb"{1,64}"
# 40 characters of encoded body, wrapped at 20 columns or more (base64 lines
# are whole groups of 4)
BODY = rb"[A-Za-z0-9+/]{20}(?:" + B0 + rb"[A-Za-z0-9+/]{4}){5}"

PATTERNS = {
    "GitHub token": rb"gh[pousr]_[A-Za-z0-9]{36}|github_pat_[A-Za-z0-9_]{60,}",
    "GitLab token": rb"glpat-[A-Za-z0-9_-]{20}",
    "npm token": rb"npm_[A-Za-z0-9]{36}",
    "PyPI token": rb"pypi-AgEI[A-Za-z0-9_-]{50,}",
    "Docker Hub token": rb"dckr_pat_[A-Za-z0-9_-]{27,}",
    "Hugging Face token": rb"hf_[A-Za-z0-9]{34}",
    "Anthropic or OpenAI key": rb"sk-ant-[A-Za-z0-9_-]{32,}|sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{40,}|sk-[A-Za-z0-9]{48}",
    "Stripe key": rb"[sr]k_live_[0-9A-Za-z]{24,}",
    "SendGrid key": rb"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}",
    "Vault token": rb"hv[sb]\.[A-Za-z0-9_-]{24,}",
    "DigitalOcean token": rb"do[por]_v1_[a-f0-9]{64}",
    "Shopify token": rb"shp(?:at|ca|pa|ss)_[a-fA-F0-9]{32}",
    "age secret key": rb"AGE-SECRET-KEY-1[0-9A-Z]{58}",
    "Google API key or OAuth token": rb"AIza[0-9A-Za-z_-]{35}|ya29\.[0-9A-Za-z_-]{30,}",
    "Slack token or webhook": rb"xox[abeprs]-[0-9A-Za-z-]{10,}|xapp-[0-9A-Za-z-]{10,}|hooks\.slack\.com/services/T[A-Z0-9]{8,}/B[A-Z0-9]{8,}/[A-Za-z0-9]{24}",
    "registry auth": rb'"(?:auth|identitytoken)"[ \t]*:[ \t]*"[A-Za-z0-9+/=._-]{16,}"',
    "AWS key ID": rb"(?<![A-Z0-9])(?:AKIA|ASIA)[0-9A-Z]{16}(?![A-Z0-9])",
    # the body after any "Name: value" headers (PEM, PGP), on the next line or
    # the same one (a key with its line breaks taken out)
    "private key": rb"-----BEGIN [A-Z ]{0,20}PRIVATE KEY(?: BLOCK)?-----" + B0
    + rb"(?:[A-Za-z-]{1,40}: [^\r\n\\]{0,200}" + B1 + rb"){0,8}" + BODY,
    # a PEM private key base64-encoded once more (kubeconfig client-key-data,
    # Kubernetes secrets): "-----BEGIN " then "PRIVATE KEY", at any alignment
    "encoded private key": rb"(?:LS0tLS1CRUdJ|LS0tQkVHSU4g|LS0tLUJFR0lO)[A-Za-z0-9+/]{0,40}"
    rb"(?:UFJJVkFURSBL|SVZBVEUgS0VZ|UklWQVRFIEtF)",
}
ALL = [(kind, re.compile(p)) for kind, p in PATTERNS.items()]

# (kind, path) pairs that are known and public: gnutls embeds the keys of its
# FIPS self-tests. Allowed only with a matching --allow-sha256.
ALLOWED = [
    ("private key", re.compile(r"/usr/lib64/libgnutls\.so\.[0-9.]+")),
]

CHUNK = 8 << 20
# longer than any match can be (the private key pattern's bound is ~4 KiB)
OVERLAP = 64 << 10

LAYER = re.compile(r"[0-9a-f]{64}\.tar")
METADATA = re.compile(r"manifest\.json|repositories|[0-9a-f]{64}\.json|[0-9a-f]{64}/(?:json|VERSION)")


def allowlisted(path):
    return any(rx.fullmatch(path) for _, rx in ALLOWED)


def allowed(kind, path, digest, digests):
    return digest in digests and any(k == kind and rx.fullmatch(path) for k, rx in ALLOWED)


def shown(name):
    """A path safe for a public log: escaped (no newline can start a
    workflow command, no line can look like one) and cut short."""
    text = name.encode("unicode_escape").decode("ascii").replace("::", ": :").replace("##[", "# #[")
    return text if len(text) <= 200 else text[:200] + "..."


class Matches:
    """Match start offsets per kind over a stream fed in pieces; a match in
    the overlap of two pieces counts once."""

    def __init__(self):
        self.offsets = {}
        self.pos = 0
        self.tail = b""

    def feed(self, block):
        data = self.tail + block
        base = self.pos - len(self.tail)
        for kind, rx in ALL:
            for m in rx.finditer(data):
                self.offsets.setdefault(kind, set()).add(base + m.start())
        self.pos += len(block)
        self.tail = data[-OVERLAP:]


class RawScan:
    """Wraps the archive stream and scans every byte read through it, in
    large pieces (tarfile reads 10 KiB at a time)."""

    def __init__(self, raw):
        self.raw = raw
        self.matches = Matches()
        self.pending = []
        self.size = 0

    def read(self, n=-1):
        block = self.raw.read(n)
        if block:
            self.pending.append(block)
            self.size += len(block)
            if self.size >= CHUNK:
                self.flush()
        return block

    def flush(self):
        if self.pending:
            self.matches.feed(b"".join(self.pending))
            self.pending = []
            self.size = 0


class Files:
    """The byte range of every file's data in the archive stream."""

    def __init__(self):
        self.starts = []
        self.ranges = []

    def add(self, start, size, label, path, digest=None):
        self.starts.append(start)
        self.ranges.append((start, start + size, label, path, digest))

    def at(self, offset):
        i = bisect.bisect_right(self.starts, offset) - 1
        if i >= 0:
            start, end, label, path, digest = self.ranges[i]
            if start <= offset < end:
                return label, path, digest
        return None


class Unexpected(Exception):
    pass


def read_archive(raw, files):
    """Records every file's range; returns the layers read and the
    manifest's list of layers."""
    layers = set()
    manifest = None
    with tarfile.open(fileobj=raw, mode="r|") as image:
        for member in image:
            if member.isdir() or member.issym():
                continue
            if not member.isfile():
                raise Unexpected("a member that is not a file, directory or symlink")
            if LAYER.fullmatch(member.name):
                layers.add(member.name)
                layer_name = shown(member.name)
                f = image.extractfile(member)
                # members come in stream order, so the starts stay sorted
                with tarfile.open(fileobj=f, mode="r|") as layer:
                    for m in layer:
                        if not m.isfile():
                            continue
                        path = posixpath.normpath("/" + m.name)
                        digest = None
                        if allowlisted(path):
                            digest = hashlib.sha256(layer.extractfile(m).read()).hexdigest()
                        files.add(member.offset_data + m.offset_data, m.size, layer_name, path, digest)
            elif METADATA.fullmatch(member.name):
                files.add(member.offset_data, member.size, None, shown(member.name))
                if member.name == "manifest.json":
                    manifest = json.loads(image.extractfile(member).read())
            else:
                raise Unexpected("an unknown member: " + shown(member.name))
    if not isinstance(manifest, list) or not manifest:
        raise Unexpected("no manifest.json")
    listed = set()
    for entry in manifest:
        listed.update(entry["Layers"])
    return layers, listed


def main(argv):
    digests = set()
    args = argv[1:]
    while args:
        if args[0] != "--allow-sha256" or len(args) < 2 or not re.fullmatch(r"[0-9a-f]{64}", args[1]):
            print("usage: scan-layers.py [--allow-sha256 HEX]... < archive", file=sys.stderr)
            return 2
        digests.add(args[1])
        args = args[2:]

    files = Files()
    raw = RawScan(sys.stdin.buffer)
    try:
        layers, listed = read_archive(raw, files)
        # read what follows the archive's end marker (and drain the pipe)
        while raw.read(CHUNK):
            pass
        raw.flush()
    except Unexpected as e:
        print(f"the image archive is not as expected: {e}", file=sys.stderr)
        return 2
    except Exception as e:  # anything: a scan that could not finish is a failure
        print(f"cannot read the image archive: {type(e).__name__}", file=sys.stderr)
        return 2
    if not layers:
        print("no layers in the image archive", file=sys.stderr)
        return 2
    if not listed <= layers:
        print(f"{len(listed - layers)} layer(s) in the manifest were not read", file=sys.stderr)
        return 2
    hits = set()
    for kind, offsets in raw.matches.offsets.items():
        for off in offsets:
            where = files.at(off)
            if where is None:
                hits.add(f"elsewhere in the archive, outside any file ({kind})")
            elif where[0] is None:
                hits.add(f"{where[1]} ({kind})")
            elif not allowed(kind, where[1], where[2], digests):
                # an allow-listed file that is not the packaged one: its digest
                # (of a whole public library, not a secret) to look into
                note = f", sha256 {where[2]} not the packaged file" if where[2] else ""
                hits.add(f"{where[0]}: {shown(where[1])} ({kind}{note})")
    for h in sorted(hits)[:20]:
        print(h)
    return 1 if hits else 0


if __name__ == "__main__":
    try:
        status = main(sys.argv)
    except Exception as e:  # never Python's own exit 1, which means "a hit"
        print(f"scan failed: {type(e).__name__}", file=sys.stderr)
        status = 2
    sys.exit(status)
