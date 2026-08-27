#!/usr/bin/env bash
#
# Checks the manifests of one version against the published winget schemas, and re-computes
# both hashes from the real downloads.
#
#   ./validate.sh 0.1.0
#
# This is what can be checked from macOS. It does NOT replace `winget validate` and
# `winget install --manifest` on a Windows machine, which are what the pipeline actually
# runs; it catches the mistakes that are worth catching before getting that far.

set -euo pipefail

VERSION="${1:-}"
if [[ -z "${VERSION}" ]]; then
  echo "Usage: ./validate.sh <version>"
  echo "Available:"
  find . -maxdepth 1 -type d -name '[0-9]*' -exec basename {} \; | sort | sed 's/^/  /'
  exit 1
fi

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/${VERSION}"
[[ -d "${DIR}" ]] || { echo "Error: ${DIR} does not exist."; exit 1; }

command -v python3 >/dev/null || { echo "Error: python3 is required."; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

echo "Preparing a throwaway environment for the checks ..."
python3 -m venv "${WORK}/venv" >/dev/null
"${WORK}/venv/bin/pip" install --quiet jsonschema pyyaml

MANIFEST_VERSION="$(grep -m1 '^ManifestVersion:' "${DIR}"/*.installer.yaml | awk '{print $2}')"
echo "Manifest schema version: ${MANIFEST_VERSION}"

for kind in version installer defaultLocale; do
  curl -sL --max-time 30 \
    "https://aka.ms/winget-manifest.${kind}.${MANIFEST_VERSION}.schema.json" \
    -o "${WORK}/${kind}.json"
done

"${WORK}/venv/bin/python" - "${DIR}" "${WORK}" <<'PY'
import glob, json, sys, hashlib, urllib.request
import yaml, jsonschema

directory, work = sys.argv[1], sys.argv[2]

class Loader(yaml.SafeLoader):
    """winget does not turn ReleaseDate into a date object and neither should this."""
Loader.add_constructor("tag:yaml.org,2002:timestamp",
                       lambda l, n: l.construct_scalar(n))

def load(path):
    with open(path, encoding="utf-8") as handle:
        return yaml.load(handle, Loader=Loader)

files = {
    "installer": glob.glob(f"{directory}/*.installer.yaml")[0],
    "defaultLocale": glob.glob(f"{directory}/*.locale.*.yaml")[0],
}
files["version"] = [
    f for f in glob.glob(f"{directory}/*.yaml")
    if f not in files.values()
][0]

failures = 0
docs = {}
for kind, path in files.items():
    doc = load(path)
    docs[kind] = doc
    schema = json.load(open(f"{work}/{kind}.json"))
    errors = sorted(jsonschema.Draft7Validator(schema).iter_errors(doc),
                    key=lambda e: list(e.path))
    name = path.rsplit("/", 1)[-1]
    if errors:
        failures += 1
        print(f"  FAIL  {name}")
        for error in errors:
            where = "/".join(map(str, error.path)) or "(root)"
            print(f"          {where}: {error.message[:150]}")
    else:
        print(f"  ok    {name}")

identifiers = {d["PackageIdentifier"] for d in docs.values()}
versions = {d["PackageVersion"] for d in docs.values()}
if len(identifiers) != 1:
    print(f"  FAIL  PackageIdentifier disagrees across files: {identifiers}")
    failures += 1
if len(versions) != 1:
    print(f"  FAIL  PackageVersion disagrees across files: {versions}")
    failures += 1
if docs["version"]["DefaultLocale"] != docs["defaultLocale"]["PackageLocale"]:
    print("  FAIL  DefaultLocale does not match the locale manifest")
    failures += 1

print()
print("  Hashes, recomputed from the real downloads:")
for installer in docs["installer"]["Installers"]:
    data = urllib.request.urlopen(installer["InstallerUrl"]).read()
    digest = hashlib.sha256(data).hexdigest().upper()
    if digest == installer["InstallerSha256"]:
        print(f"    ok    {installer['Architecture']:6} {len(data):>7} bytes")
    else:
        failures += 1
        print(f"    FAIL  {installer['Architecture']:6} manifest says {installer['InstallerSha256']}")
        print(f"                 download is  {digest}")

print()
if failures:
    print(f"{failures} problem(s). Do not submit this.")
    sys.exit(1)
print("Everything checks out. Still run `winget validate` and `winget install --manifest`")
print("on Windows before opening the pull request.")
PY
