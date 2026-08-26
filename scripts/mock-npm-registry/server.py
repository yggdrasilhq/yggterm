#!/usr/bin/env python3
"""A tiny mock npm registry for the managed-CLI install harness.

Serves, from a fixture root, exactly the three surfaces the direct fetcher
consumes:

    GET /<pkg>/<tag-or-version>   -> manifest JSON (version, dist.tarball,
                                     optionalDependencies, scripts, bin)
    GET /<pkg>/-/<file>.tgz       -> the tarball

Fixture layout (built by the test that spawns this server):

    <root>/packages/<name>/
        tag                    # the dist-tag `latest`/`beta` resolves to
        v1/                    # a version directory = one publishable version
            package.json       # manifest fragment (version/bin/scripts/...)
            files/             # files the tarball contains (relative paths)
        v2/ ...

The tarball for version V of package P is a gzipped tar of
`packages/P/V/files/` under `package/`. Manifests are composed from
`package.json` plus `dist.tarball` pointing back at THIS server.

Run:  server.py <root> <port>   (stdout: nothing; exit 0 on ready log to stderr)
"""
import gzip
import io
import json
import os
import sys
import tarfile
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = None


def compose_manifest(pkg, ref):
    pkg_dir = os.path.join(ROOT, "packages", pkg)
    if not os.path.isdir(pkg_dir):
        return None
    versions = sorted(
        d for d in os.listdir(pkg_dir) if os.path.isdir(os.path.join(pkg_dir, d))
    )
    if not versions:
        return None
    version = None
    if os.path.isdir(os.path.join(pkg_dir, ref)):
        version = ref
    elif os.path.isfile(os.path.join(pkg_dir, "tag")):
        tagged = open(os.path.join(pkg_dir, "tag")).read().strip()
        if tagged in versions:
            version = tagged
    if version is None:
        version = versions[-1]
    version_dir = os.path.join(pkg_dir, version)
    files_dir = os.path.join(version_dir, "files")
    fragment = {}
    fragment_path = os.path.join(version_dir, "package.json")
    if os.path.isfile(fragment_path):
        fragment = json.load(open(fragment_path))
    name_in_tar = f"{pkg.split('/')[-1]}-{version}.tgz"
    manifest = {
        "name": pkg,
        "version": version,
        "dist": {"tarball": f"http://{HOST_HEADER}/{pkg}/-/{name_in_tar}"},
    }
    manifest.update(fragment)
    return manifest


def build_tarball(pkg, version):
    version_dir = os.path.join(ROOT, "packages", pkg, version)
    files_dir = os.path.join(version_dir, "files")
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        if os.path.isdir(files_dir):
            for base, _dirs, names in os.walk(files_dir):
                for name in names:
                    full = os.path.join(base, name)
                    rel = os.path.relpath(full, files_dir)
                    info = tarfile.TarInfo(f"package/{rel}")
                    data = open(full, "rb").read()
                    info.size = len(data)
                    mode = os.stat(full).st_mode & 0o777
                    info.mode = mode
                    tar.addfile(info, io.BytesIO(data))
    return buf.getvalue()


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        path = self.path.lstrip("/")
        # /<pkg>/-/<file>.tgz
        if "/-/" in path and path.endswith(".tgz"):
            pkg_part, file_part = path.split("/-/", 1)
            pkg = pkg_part
            stem = file_part[:-4]
            # <name>-<version>.tgz  (version may itself contain dashes)
            version = stem[len(pkg.split("/")[-1]) + 1:]
            data = build_tarball(pkg, version)
            self.send_response(200)
            self.send_header("Content-Type", "application/gzip")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            return
        # /<pkg>/<tag-or-version>
        parts = path.split("/", 1)
        if len(parts) == 2:
            pkg, ref = parts
            manifest = compose_manifest(pkg, ref)
            if manifest is not None:
                data = json.dumps(manifest).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)
                return
        self.send_response(404)
        self.end_headers()


if __name__ == "__main__":
    ROOT = os.path.abspath(sys.argv[1])
    port = int(sys.argv[2])
    HOST_HEADER = f"127.0.0.1:{port}"
    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"ready {port}", flush=True)
    server.serve_forever()
