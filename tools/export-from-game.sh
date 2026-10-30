#!/usr/bin/env bash
# Export the engine crates from the game repository (Blue Water) into this one.
#
#   tools/export-from-game.sh <game-checkout> [git-ref]        # git-ref defaults to origin/master
#
# It takes exactly the files committed at <git-ref> (git archive, never the checkout's working tree), keeps the game's
# directory layout (so relative paths and native-unify's paths keep working), then trims what a fresh clone of this
# repository cannot build: dev-dependencies on game crates that are not exported, the workspaces' entries for absent
# crates, and native-unify's entries for packages only the game uses. Re-run it after a change lands in the game: it
# replaces every exported path and leaves README.md (but for its commit line), tools/, .gitignore and the Cargo targets
# alone. It reads the game checkout and never changes it.
#
# Needs git, tar, python3 (3.11+, for tomllib) and cargo (with the registry cache, or a network, for `cargo tree` and
# the lockfiles' pruning).
set -euo pipefail

# ---- What goes in: paths in the game repository, exported at the same paths. Extend these lists. ---------------------

# The root workspace (crates/): portable crates the native ones depend on. These become the root workspace's members.
ROOT_CRATES=(
  crates/naval-sky-types
  crates/naval-cloud-field
  crates/naval-land-format
  crates/naval-earth
)

# The native workspace (native/crates/, members by its `crates/*` glob).
NATIVE_CRATES=(
  native/crates/naval-render
  native/crates/naval-sky
  native/crates/naval-clouds
  native/crates/naval-conditions
  native/crates/naval-ocean
  native/crates/naval-water-effects
  native/crates/naval-effects
  native/crates/naval-terrain
  native/crates/naval-land
  native/crates/naval-region-bake
  native/crates/naval-osm
  native/crates/naval-overture
  native/crates/naval-land-bake
  native/crates/naval-globe
  native/crates/naval-landscape
  native/crates/naval-vessel
  native/crates/naval-afloat
  native/crates/engine-example
  native/crates/native-unify
  native/crates/native-offscreen
)

# Files the crates read outside themselves, the patched wgpu-hal and the toolchain.
OTHER_PATHS=(
  native/vendor                          # wgpu-hal 30 with a larger DX12 shader cache: the [patch.crates-io] target
  assets/maps/environments.v1.json       # naval-conditions: the maps (compiled in)
  assets/maps/battle-conditions.v1.json  # naval-conditions: the weather presets (compiled in)
  assets/bases/bases.v1.json             # naval-terrain: the naval bases' places (compiled in)
  assets/land                            # naval-region-bake's tests: the committed land releases' files and tables
  public/terrain                         # naval-terrain's ground photographs (Poly Haven CC0, public/terrain/ASSETS.md)
  assets/effects/flipbook                # naval-effects' game look: smoke, fire and explosion simulations (JangaFX EmberGen CC0; Git LFS in the game)
  public/theatre/world/world-v1.bin      # naval-globe's world chart, read by engine-example (Natural Earth, public domain)
  src/ui/tokens.css                      # naval-render: the palette test reads the interface's colour tokens
  rust-toolchain.toml
)

# Docs for these crates. Their links into the game's other docs stay broken. A path absent at the ref is skipped.
DOCS=(
  native/docs/sky.md
  native/docs/haze.md
  native/docs/sea-sky.md
  native/docs/water-effects.md
  native/docs/frame-hygiene.md
  native/docs/clouds
  native/docs/ocean
  native/docs/terrain
  native/docs/combat-effects
  native/docs/render-core
  native/docs/lighting
  native/docs/vessel.md
  native/docs/afloat.md
)

# The workspaces' manifests and lockfiles: exported, then rewritten below.
WORKSPACE_FILES=(Cargo.toml Cargo.lock native/Cargo.toml native/Cargo.lock)

# Game packages whose part of the dependency graph native-unify's filter leaves out (they reach the exported crates
# only through the dev-dependencies removed below).
GAME_ONLY_PACKAGES=(naval-sim naval-protocol naval-sim-host)

# ---- Arguments ---------------------------------------------------------------------------------------------------------

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <game-checkout> [git-ref]" >&2
  exit 2
fi
GAME=$(cd "$1" && pwd)
REF=${2:-origin/master}
DEST=$(cd "$(dirname "$0")/.." && pwd)
SHA=$(git -C "$GAME" rev-parse --verify "$REF^{commit}")
echo "export: $GAME at $REF ($SHA) -> $DEST"

exists_at_ref() { [[ -n $(git -C "$GAME" ls-tree --name-only "$SHA" -- "$1") ]]; }

PATHS=()
for p in "${ROOT_CRATES[@]}" "${NATIVE_CRATES[@]}" "${OTHER_PATHS[@]}" "${WORKSPACE_FILES[@]}"; do
  exists_at_ref "$p" || { echo "export: $p is not in $SHA" >&2; exit 1; }
  PATHS+=("$p")
done
for p in "${DOCS[@]}"; do
  if exists_at_ref "$p"; then PATHS+=("$p"); else echo "export: skipped $p (not in $SHA)"; fi
done

# ---- Clean the previous export and extract -----------------------------------------------------------------------------

# Everything under these is the export's; native/target (a Cargo target) is kept.
rm -rf "$DEST/crates" "$DEST/assets" "$DEST/public" "$DEST/src" "$DEST/Cargo.toml" "$DEST/Cargo.lock" \
  "$DEST/rust-toolchain.toml"
if [[ -d $DEST/native ]]; then
  find "$DEST/native" -mindepth 1 -maxdepth 1 ! -name target -exec rm -rf {} +
fi
git -C "$GAME" archive "$SHA" -- "${PATHS[@]}" | tar -x -C "$DEST"

# git archive writes a Git LFS file's content when the game's checkout holds its object, else its pointer.
POINTERS=$(grep -rlI --exclude-dir=target '^version https://git-lfs.github.com/spec/' "$DEST/assets" "$DEST/public" || true)
if [[ -n $POINTERS ]]; then
  echo "export: Git LFS pointers in place of files (git lfs pull in the game's checkout):" >&2
  echo "$POINTERS" >&2
  exit 1
fi

# ---- Trims: dev-dependencies on game crates that are not exported, and the targets that need them --------------------
#
# naval-water-effects: naval-protocol, naval-sim, naval-sim-host (a recorded battle drives the effects)
#   - dev-dependencies naval-sim-host, naval-sim, naval-protocol
#   - examples/recorder/ (records a battle with the simulation), examples/record.rs, examples/battle_sequence.rs
#   - tests/fixture_playback.rs (and `#[path] mod recorder;`, `mod fixture_playback;` in tests/all.rs)
N=$DEST/native/crates
rm -rf "$N/naval-water-effects/examples/recorder"
rm -f "$N/naval-water-effects/examples/record.rs" "$N/naval-water-effects/examples/battle_sequence.rs" \
  "$N/naval-water-effects/tests/fixture_playback.rs"

ROOT_CRATES_CSV=$(IFS=,; echo "${ROOT_CRATES[*]}")
python3 -I - "$DEST" "$ROOT_CRATES_CSV" <<'PY'
import re, sys
from pathlib import Path

dest = Path(sys.argv[1])
root_crates = sys.argv[2].split(",")
native = dest / "native" / "crates"

def read(p): return p.read_text().split("\n")
def write(p, lines): p.write_text("\n".join(lines))

def comment_block_start(lines, i):
    """The first line of the comment block directly above line i (i itself when there is none)."""
    while i > 0 and lines[i - 1].lstrip().startswith("#"):
        i -= 1
    return i

def remove_dep(manifest, table, name):
    """Remove `name` from `[table]` and the comment block directly above it."""
    lines = read(manifest)
    section, hits = None, []
    for i, line in enumerate(lines):
        m = re.match(r"^\[(.+)\]\s*$", line)
        if m:
            section = m.group(1).strip()
            continue
        if section == table and re.match(rf"^{re.escape(name)}(\.workspace)?\s*=", line):
            hits.append(i)
    if len(hits) != 1:
        sys.exit(f"export: {manifest}: [{table}] {name} found {len(hits)} times")
    i = hits[0]
    del lines[comment_block_start(lines, i):i + 1]
    write(manifest, lines)

def remove_lines(path, exact):
    """Remove each line in `exact` (each must occur once), then collapse doubled blank lines."""
    lines = read(path)
    for e in exact:
        n = sum(1 for l in lines if l == e)
        if n != 1:
            sys.exit(f"export: {path}: {e!r} found {n} times")
        lines.remove(e)
    out = []
    for l in lines:
        if l.strip() == "" and out and out[-1].strip() == "":
            continue
        out.append(l)
    write(path, out)

wfx = native / "naval-water-effects"
for dep in ("naval-sim-host", "naval-sim", "naval-protocol"):
    remove_dep(wfx / "Cargo.toml", "dev-dependencies", dep)
remove_lines(wfx / "tests" / "all.rs",
             ['#[path = "../examples/recorder/mod.rs"]', "mod recorder;", "mod fixture_playback;"])

# ---- The workspaces: members present, [workspace.dependencies] path entries only for crates present ----------------

def drop_absent_paths(manifest, base):
    """Drop [workspace.dependencies] path entries whose crate is absent, with the comment block above an entry unless
    the line after it is a kept dependency (the comment then describes that one too)."""
    lines = read(manifest)
    section, drop = None, set()
    for i, line in enumerate(lines):
        m = re.match(r"^\[(.+)\]\s*$", line)
        if m:
            section = m.group(1).strip()
        p = re.search(r'[{,]\s*path\s*=\s*"([^"]+)"', line)
        if section == "workspace.dependencies" and p and not (base / p.group(1)).exists():
            drop.add(i)
    out, dropped = [], []
    for i, line in enumerate(lines):
        if i in drop:
            j = i + 1
            while j in drop:
                j += 1
            if not (j < len(lines) and re.match(r"^[A-Za-z0-9_-]+\s*=", lines[j])):
                del out[comment_block_start(out, len(out)):]
            dropped.append(line.split("=")[0].strip())
            continue
        out.append(line)
    write(manifest, out)
    print(f"export: {manifest.relative_to(dest)}: dropped workspace entries {', '.join(dropped) or '(none)'}")

root = dest / "Cargo.toml"
text = root.read_text()
members = "members = [\n" + "".join(f'    "{c}",\n' for c in root_crates) + "]"
text, n = re.subn(r"members = \[[^\]]*\]", members, text, count=1)
if n != 1:
    sys.exit("export: Cargo.toml: no members list")
root.write_text(text)
drop_absent_paths(root, dest)
drop_absent_paths(dest / "native" / "Cargo.toml", dest / "native")
PY

# ---- native-unify: only the entries for packages in the exported crates' dependency graph ------------------------------
#
# `cargo tree` runs in a copy of the game's workspaces at the ref (so the game checkout is only read), rooted at the
# exported native crates, with native-unify pruned (its own entries would otherwise count) and the game-only packages
# above pruned, over every target platform and normal, build and dev edges.
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
git -C "$GAME" archive "$SHA" -- Cargo.toml Cargo.lock rust-toolchain.toml crates native/Cargo.toml native/Cargo.lock \
  native/crates native/vendor | tar -x -C "$WORK"
TREE_ARGS=(tree --locked -e normal,build,dev --target all --prefix none -f '{p}' --prune native-unify)
for p in "${GAME_ONLY_PACKAGES[@]}"; do TREE_ARGS+=(--prune "$p"); done
for c in "${NATIVE_CRATES[@]}"; do
  name=${c##*/}
  [[ $name == native-unify ]] || TREE_ARGS+=(-p "$name")
done
if ! (cd "$WORK/native" && cargo "${TREE_ARGS[@]}" --offline > "$WORK/graph.txt" 2> "$WORK/tree.log"); then
  (cd "$WORK/native" && cargo "${TREE_ARGS[@]}" > "$WORK/graph.txt" 2> "$WORK/tree.log") || { cat "$WORK/tree.log" >&2; exit 1; }
fi

python3 -I - "$DEST" "$WORK/graph.txt" <<'PY'
import re, sys, tomllib
from pathlib import Path

dest, graph = Path(sys.argv[1]), Path(sys.argv[2])
versions = {}
for line in graph.read_text().splitlines():
    parts = line.split()
    if len(parts) >= 2 and parts[1].startswith("v"):
        versions.setdefault(parts[0], set()).add(parts[1][1:])

manifest = dest / "native" / "crates" / "native-unify" / "Cargo.toml"
out, kept, dropped, section = [], 0, set(), None
for line in manifest.read_text().split("\n"):
    m = re.match(r"^\[(.+)\]\s*$", line)
    if m:
        section = m.group(1)
    if section and "dependencies" in section and re.match(r"^[A-Za-z0-9_-]+\s*=\s*\{", line):
        key, spec = next(iter(tomllib.loads(line).items()))
        name = spec.get("package", key)
        have = versions.get(name, set())
        if have and ("path" in spec or spec.get("version") in have):
            kept += 1
        else:
            dropped.add(f"{name} {spec.get('version', '(path)')}")
            continue
    out.append(line)

note = ("# Filtered by tools/export-from-game.sh: only the entries for packages in the exported crates' dependency graph\n"
        "# (`cargo tree` in the game's workspace with native-unify pruned); the game's other dependencies are left out.")
text = "\n".join(out)
first_newline = text.index("\n")
text = text[:first_newline + 1] + note + "\n" + text[first_newline + 1:]
manifest.write_text(text)
print(f"export: native-unify: kept {kept} entries, dropped {len(dropped)} packages: {', '.join(sorted(dropped))}")
PY

# ---- Every path dependency resolves in the export ----------------------------------------------------------------------

python3 -I - "$DEST" <<'PY'
import sys, tomllib
from pathlib import Path

dest = Path(sys.argv[1])
def workspace_deps(ws):
    return tomllib.loads((ws / "Cargo.toml").read_text()).get("workspace", {}).get("dependencies", {})
workspaces = {dest: workspace_deps(dest), dest / "native": workspace_deps(dest / "native")}

def tables(doc):
    for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
        yield kind, doc.get(kind, {})
    for target, t in doc.get("target", {}).items():
        for kind in ("dependencies", "dev-dependencies", "build-dependencies"):
            yield f"target.{target}.{kind}", t.get(kind, {})

bad = []
manifests = sorted(dest.glob("crates/*/Cargo.toml")) + sorted(dest.glob("native/crates/*/Cargo.toml"))
for m in manifests:
    ws = dest / "native" if "native" in m.relative_to(dest).parts else dest
    doc = tomllib.loads(m.read_text())
    for kind, deps in tables(doc):
        for name, spec in deps.items():
            if isinstance(spec, dict) and spec.get("workspace"):
                if name not in workspaces[ws]:
                    bad.append(f"{m.relative_to(dest)} [{kind}] {name}: not in {ws.relative_to(dest) or '.'}/Cargo.toml")
                    continue
                spec, base = workspaces[ws][name], ws
            else:
                base = m.parent
            if isinstance(spec, dict) and "path" in spec and not (base / spec["path"]).exists():
                bad.append(f"{m.relative_to(dest)} [{kind}] {name}: {spec['path']} is not exported")
if bad:
    sys.exit("export: dependencies on crates that are not exported:\n  " + "\n  ".join(bad))
print(f"export: every dependency of {len(manifests)} crates resolves in the export")
PY

# ---- Files the crates compile in resolve in the export (include_str!/include_bytes! with a literal path) --------------

python3 -I - "$DEST" <<'PY'
import re, sys
from pathlib import Path

dest = Path(sys.argv[1])
pattern = re.compile(r'include_(?:str|bytes)!\(\s*"([^"]+)"\s*\)')
missing = []
for rs in list(dest.glob("crates/**/*.rs")) + list(dest.glob("native/crates/**/*.rs")):
    if "target" in rs.relative_to(dest).parts:
        continue
    for m in pattern.finditer(rs.read_text(errors="replace")):
        if not (rs.parent / m.group(1)).exists():
            missing.append(f"{rs.relative_to(dest)}: {m.group(1)}")
if missing:
    sys.exit("export: compiled-in files that are not exported:\n  " + "\n  ".join(missing))
print("export: every include_str!/include_bytes! path resolves")
PY

# ---- Lockfiles: drop the packages no exported crate uses (versions unchanged) -------------------------------------------

prune_lock() {
  local dir=$1
  cp "$dir/Cargo.lock" "$WORK/before.lock"
  if ! (cd "$dir" && cargo metadata --format-version 1 --offline > /dev/null 2> "$WORK/metadata.log"); then
    (cd "$dir" && cargo metadata --format-version 1 > /dev/null 2> "$WORK/metadata.log") || { cat "$WORK/metadata.log" >&2; exit 1; }
  fi
  python3 -I - "$WORK/before.lock" "$dir/Cargo.lock" <<'PY'
import sys, tomllib
before, after = (tomllib.loads(open(p).read())["package"] for p in sys.argv[1:3])
key = lambda p: (p["name"], p["version"])
old = {key(p) for p in before}
new = {key(p) for p in after}
added = new - old
if added:
    sys.exit(f"export: {sys.argv[2]}: pruning changed versions: {sorted(added)}")
print(f"export: {sys.argv[2]}: {len(new)} packages ({len(old - new)} pruned)")
PY
}
prune_lock "$DEST"
prune_lock "$DEST/native"

# ---- The README's commit line ------------------------------------------------------------------------------------------

if [[ -f $DEST/README.md ]]; then
  python3 -I - "$DEST/README.md" "$SHA" <<'PY'
import re, sys
path, sha = sys.argv[1], sys.argv[2]
text = open(path).read()
text, n = re.subn(r"(<!-- exported-commit -->)`[0-9a-f]*`", rf"\g<1>`{sha}`", text)
open(path, "w").write(text)
print(f"export: README.md commit line {'updated' if n else 'not found'}")
PY
fi

echo "export: done ($SHA)"
