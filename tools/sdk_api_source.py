#!/usr/bin/env python3
"""The SDK's API source: the mod API crates as a workspace of their own, with their generated code written out, so
a mod builds against them with no game source and no generator run.

    python3 tools/sdk_api_source.py build OUT    # write the API source for this checkout's commit into OUT
    python3 tools/sdk_api_source.py check OUT    # check the starter mod against OUT alone, host and wasm32
    python3 tools/sdk_api_source.py archive OUT ARCHIVE  # OUT as the SDK's api-source.tar.xz (its root is OUT's)

`build` takes the generated files from the in-tree build scripts' own output (`cargo check` for the host, for
wasm32 and with every feature the API source keeps) and refuses when they differ between those builds. Each crate
carries exactly the source files rustc read for its library (its dep-info), its manifest with workspace keys
resolved and no dev- or build-dependencies, and, for a crate whose build script runs the generator, a build script
that copies `generated/` into `OUT_DIR`. The generated files name source files by absolute path; the checkout's
root is written as ROOT_TOKEN and the build script puts the API source's own root in its place. `build` runs on a
POSIX host (CI's Linux leg): it relocates `/`-separated paths only. `Cargo.lock` pins
the versions the game's lockfile has. Native feature unions come from each published SDK target's actual
normal-library Cargo graph, in separate target dependency tables; wasm keeps its original dependency model.
No package outside either API root's runtime closure is added. Same commit, same bytes.
"""

import argparse
import copy
import gzip
import hashlib
import io
import json
import lzma
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

# Beside this file: a module of the `tools` package when imported, a plain module when run as a script.
if __package__:
    from . import prebuilt_game
else:
    import prebuilt_game

ROOT = Path(__file__).resolve().parent.parent
API = "sunrise-mod-api"
# The game's one wasmtime, a crate of its own beside the API: native mods only, so no wasm32 build of it.
WASMTIME = "sunrise-wasmtime"
WASM = "wasm32-unknown-unknown"
GENERATED = "generated"
ROOT_TOKEN = "{sunrise-api-source-root}"
TEMPLATE = ROOT / "crates/cargo-sunrise-mod/template/native"
# Where a mod finds the API source (design: the SDK fetched into the mod's `.sunrise/sdk/`).
MOD_PATH = ".sunrise/sdk/api"
# Marks a scratch folder `check` made, which a later check may empty.
SCRATCH_MARK = ".sdk-api-source-check"
# The build script of a crate whose generated files come with it. Paths become `/`-separated, valid in Rust
# string literals on every host.
BUILD_SCRIPT = f"""\
//! Writes the generated files that come with this crate into `OUT_DIR`, naming this workspace's root where
//! they name source files.
use std::{{env, fs, path::PathBuf}};

fn main() {{
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR"));
    let root = manifest.join("../..");
    let root = root.to_str().expect("a UTF-8 path").replace('\\\\', "/");
    println!("cargo:rerun-if-changed={GENERATED}");
    for entry in fs::read_dir(manifest.join("{GENERATED}")).expect("reading {GENERATED}/") {{
        let path = entry.expect("reading {GENERATED}/").path();
        let code = fs::read_to_string(&path).expect("reading a generated file");
        let name = path.file_name().expect("a file name");
        fs::write(out.join(name), code.replace("{ROOT_TOKEN}", &root)).expect("writing OUT_DIR");
    }}
}}
"""


class Refusal(Exception):
    """The API source cannot be written as asked; `code` names the rule."""

    def __init__(self, code: str, detail: str):
        super().__init__(f"{code}: {detail}")
        self.code = code


@dataclass(frozen=True)
class Crate:
    name: str
    directory: str  # relative to the workspace root, `/`-separated: the same place in the API source
    generated: bool  # its build script runs the generator: the API source carries the output instead


def run(command: list[str], **kwargs) -> str:
    print("+", " ".join(command), flush=True)
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, **kwargs).stdout


def closure(metadata: dict, roots: tuple[str, ...] = (API,)) -> list[Crate]:
    """The workspace crates the roots' libraries need: their normal dependencies, transitively, proc macros and
    theirs included. Build-dependencies are not followed: a crate's build script either runs the generator, whose
    output the API source carries, or is refused."""
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    start = [package["id"] for package in metadata["packages"] if package["name"] in roots and package["source"] is None]
    if len(start) != len(roots):
        raise Refusal("E_API_ROOT", f"expected one workspace package named each of {', '.join(roots)}")
    workspace = Path(metadata["workspace_root"])
    pending, seen, crates = start, set(), []
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        package = packages[package_id]
        if package["source"] is not None:
            continue
        build = any("custom-build" in target["kind"] for target in package["targets"])
        directory = Path(package["manifest_path"]).parent.relative_to(workspace).as_posix()
        if build and not any(dependency["kind"] == "build" and dependency.get("path")
                             and dependency["name"] == "sunrise-mod-api-gen" for dependency in package["dependencies"]):
            raise Refusal("E_BUILD_SCRIPT", f"{package['name']} has a build script that is not the generator")
        if build and directory.count("/") != 1:
            # The build script names the workspace root as `../..` from the crate.
            raise Refusal("E_LAYOUT", f"{package['name']} is at {directory}, not two folders below the root")
        crates.append(Crate(package["name"], directory, build))
        pending.extend(dependency["pkg"] for dependency in nodes[package_id]["deps"]
                       if any(kind["kind"] is None for kind in dependency["dep_kinds"]))
    return sorted(crates, key=lambda crate: crate.name)


def kept_features(manifests: dict[str, dict]) -> dict[str, dict[str, list[str]]]:
    """Each crate's features that work without the dropped dependencies: a feature that names a dev- or
    build-dependency, or a feature dropped elsewhere, is dropped, until nothing changes."""
    kept = {name: dict(manifest.get("features", {})) for name, manifest in manifests.items()}
    dependencies = {name: set(normal_dependencies(manifest)) for name, manifest in manifests.items()}
    changed = True
    while changed:
        changed = False
        for name, features in kept.items():
            for feature, values in list(features.items()):
                if not all(feature_value_kept(value, name, dependencies, kept, manifests) for value in values):
                    del features[feature]
                    changed = True
    return kept


def feature_value_kept(value: str, crate: str, dependencies: dict, kept: dict, manifests: dict) -> bool:
    if value.startswith("dep:"):
        return value[4:] in dependencies[crate]
    if "/" not in value:
        # A bare name is a feature or, for an optional dependency, its implicit feature.
        return value in kept[crate] or value in dependencies[crate]
    dependency, feature = value.split("/", 1)
    dependency = dependency.rstrip("?")
    if dependency not in dependencies[crate]:
        return False
    package = normal_dependencies(manifests[crate])[dependency].get("package", dependency)
    return package not in kept or feature in kept[package]


def normal_dependencies(manifest: dict) -> dict[str, dict]:
    """`[dependencies]` and every `[target.*.dependencies]`, by the name the crate uses."""
    return {name: spec if isinstance(spec, dict) else {"version": spec}
            for _, entries in dependency_tables(manifest) for name, spec in entries.items()}


def dependency_tables(manifest: dict):
    """Normal dependency tables without losing their target conditions or renamed keys."""
    yield None, manifest.get("dependencies", {})
    for platform, entries in sorted(manifest.get("target", {}).items()):
        yield platform, entries.get("dependencies", {})


def library_names(package: dict) -> set[str]:
    kinds = ("lib", "rlib", "dylib")
    return {target["name"].replace("-", "_") for target in package["targets"]
            if any(kind in kinds for kind in target["kind"])}


def resolve_workspace(value, inherited: dict, what: str):
    if isinstance(value, dict) and value.get("workspace") is True:
        if what not in inherited:
            raise Refusal("E_WORKSPACE_KEY", f"{what} is inherited but the workspace does not set it")
        base = inherited[what]
        rest = {key: item for key, item in value.items() if key != "workspace"}
        if not rest:
            return base
        base = base if isinstance(base, dict) else {"version": base}
        return {**base, **rest, "features": sorted(set(base.get("features", [])) | set(rest.get("features", [])))}
    return value


def normalise_manifest(manifest: dict, workspace: dict, crate: Crate, crates: dict[str, Crate],
                       features: dict[str, list[str]]) -> str:
    """The crate's manifest as the API source carries it: workspace keys resolved, path dependencies pointing
    inside the API source, no dev- or build-dependencies, no targets but the library. Source dependency
    specifications stay portable; native feature unification is written separately for every published target,
    never copied unconditionally from the computer running this tool."""
    inherited_package = workspace.get("workspace", {}).get("package", {})
    inherited_dependencies = workspace.get("workspace", {}).get("dependencies", {})
    package = {key: resolve_workspace(manifest["package"][key], inherited_package, key)
               for key in ("name", "version", "edition", "rust-version", "license", "description", "publish")
               if key in manifest["package"]}
    package.update(autobins=False, autoexamples=False, autotests=False, autobenches=False,
                   build="build.rs" if crate.generated else False)
    out = [table("package", package)]
    if "lib" in manifest:
        out.append(table("lib", {key: manifest["lib"][key] for key in ("name", "path", "proc-macro")
                                 if key in manifest["lib"]}))

    def dependencies(platform: str | None, entries: dict) -> dict:
        written = {}
        for name, spec in entries.items():
            spec = resolve_workspace(spec, inherited_dependencies, name)
            if not isinstance(spec, dict):
                written[name] = spec
                continue
            spec = dict(spec)
            if "path" in spec:
                target = crates.get(spec.get("package", name))
                if target is None:
                    raise Refusal("E_PATH_DEPENDENCY", f"{crate.name} depends on {name}, outside the API source")
                spec["path"] = os.path.relpath(target.directory, crate.directory).replace(os.sep, "/")
            written[name] = spec
        return written

    for platform, entries in dependency_tables(manifest):
        if platform is None or entries:
            name = "dependencies" if platform is None else f"target.{json.dumps(platform)}.dependencies"
            out.append(table(name, dependencies(platform, entries)))
    if features:
        out.append(table("features", features))
    lints = manifest.get("lints", {})
    if lints.get("workspace") is True:
        lints = workspace.get("workspace", {}).get("lints", {})
    for tool, levels in sorted(lints.items()):
        out.append(table(f"lints.{tool}", levels))
    return "\n".join(out)


def toml_value(value) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, str)):
        return json.dumps(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{toml_key(key)} = {toml_value(item)}" for key, item in value.items()) + " }"
    raise Refusal("E_MANIFEST_VALUE", f"cannot write {value!r}")


def toml_key(key: str) -> str:
    return key if re.fullmatch(r"[A-Za-z0-9_-]+", key) else json.dumps(key)


def table(name: str, entries: dict) -> str:
    return f"[{name}]\n" + "".join(f"{toml_key(key)} = {toml_value(value)}\n" for key, value in entries.items())


def target_build(tree: str, *, complete: bool = False) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    """Actual target features and normal library edges. Proc macros and their host trees are excluded.
    Complete graphs require `cargo tree --no-dedupe`; metadata may retain inactive dep_kinds on surviving edges."""
    features: dict[str, set[str]] = {}
    dependencies: dict[str, set[str]] = {}
    parents: list[tuple[int, str]] = []
    host_depth = None
    pattern = re.compile(r"(\d+)(\S+) v(\S+)(.*?)\|(\S*?)( \(\*\))?")
    for line in tree.splitlines():
        match = pattern.fullmatch(line)
        if match is None:
            if complete and line.strip():
                raise Refusal("E_FEATURES", f"unreadable Cargo tree row: {line}")
            continue
        depth = int(match[1])
        if host_depth is not None and depth > host_depth:
            continue
        while parents and parents[-1][0] >= depth:
            parents.pop()
        host_depth = depth if "(proc-macro)" in match[4] else None
        if host_depth is not None:
            continue
        if complete and match[6]:
            raise Refusal("E_FEATURES", "the Cargo tree is deduplicated; dependency edges are incomplete")
        key = f"{match[2]} {match[3]}"
        features.setdefault(key, set()).update(filter(None, match[5].split(",")))
        dependencies.setdefault(key, set())
        if parents:
            if complete and parents[-1][0] != depth - 1:
                raise Refusal("E_FEATURES", "the Cargo tree skips a dependency parent")
            dependencies[parents[-1][1]].add(key)
        elif complete and depth != 0:
            raise Refusal("E_FEATURES", "the Cargo tree has no root")
        parents.append((depth, key))
    return features, dependencies


def target_features(tree: str) -> dict[str, set[str]]:
    return target_build(tree)[0]


def resolved_runtime_packages(metadata: dict, root: str, features: dict[str, set[str]],
                              dependencies: dict[str, set[str]]) -> set[str]:
    """The actual target's normal-library closure, including unified transitive features.
    The complete Cargo tree is authoritative for active edges; metadata supplies exact package identities."""
    packages: dict[str, list[dict]] = {}
    roots = []
    for package in metadata["packages"]:
        if library_names(package):
            packages.setdefault(f"{package['name']} {package['version']}", []).append(package)
        if package["name"] == root and package["source"] is None:
            roots.append(f"{package['name']} {package['version']}")
    if len(roots) != 1:
        raise Refusal("E_API_ROOT", f"expected one workspace package named {root}")
    pending = roots
    visited, runtime = set(), set()
    while pending:
        key = pending.pop()
        if key in visited:
            continue
        visited.add(key)
        if key not in features or key not in dependencies:
            raise Refusal("E_FEATURES", f"the actual build does not resolve {key}, required by {root}")
        identities = packages.get(key, [])
        if len(identities) != 1:
            raise Refusal("E_FEATURES", f"missing or ambiguous Cargo package identity for {key}")
        runtime.add(identities[0]["id"])
        pending.extend(dependencies[key])
    return runtime


def feature_unifiers(metadata: dict, root: str, features: dict[str, set[str]],
                     dependencies: dict[str, set[str]]) -> dict[str, dict]:
    """Exact existing registry pins that reproduce the game's transitive feature unions for this root.
    A dependency not in the actual root closure is never added. No dependency source is opened or copied."""
    closure = resolved_runtime_packages(metadata, root, features, dependencies)
    root_package = next(package for package in metadata["packages"]
                        if package["name"] == root and package["source"] is None)
    root_node = next((node for node in metadata["resolve"]["nodes"] if node["id"] == root_package["id"]), None)
    if root_node is None:
        raise Refusal("E_FEATURES", f"missing Cargo dependency identities for {root}")
    direct = dependencies[f"{root_package['name']} {root_package['version']}"]
    result = {}
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"], p["id"])):
        if package["id"] not in closure or package["source"] is None:
            continue
        if package["source"] != "registry+https://github.com/rust-lang/crates.io-index":
            raise Refusal("E_FEATURES", f"cannot reproduce the registry identity of {package['id']}")
        enabled = features[f"{package['name']} {package['version']}"]
        if enabled - package["features"].keys():
            raise Refusal("E_FEATURES", f"unknown resolved features of {package['id']}")
        if f"{package['name']} {package['version']}" in direct:
            # Cargo forbids one package version under two dependency names. Reuse the source declaration's
            # name, resolving renamed versions through the actual extern/package identity, not package order.
            externs = {edge["name"] for edge in root_node["deps"] if edge["pkg"] == package["id"]
                       and any(kind["kind"] is None for kind in edge["dep_kinds"])}
            names = {dependency.get("rename") or dependency["name"] for dependency in root_package["dependencies"]
                     if dependency["kind"] is None and dependency["name"] == package["name"]
                     and ((dependency.get("rename") is not None
                           and dependency["rename"].replace("-", "_") in externs)
                          or (dependency.get("rename") is None and library_names(package).intersection(externs)))}
            if len(externs) != 1 or len(names) != 1:
                raise Refusal("E_FEATURES", f"ambiguous direct dependency name for {package['id']} in {root}")
            alias = names.pop()
        else:
            # Distinct transitive versions cannot capture one another's extern. The stable identity hash
            # contains no checkout path (workspace packages are not feature-unified here).
            alias = "__sunrise_sdk_features_" + hashlib.sha256(package["id"].encode()).hexdigest()[:16]
        spec = {"package": package["name"], "version": "=" + package["version"],
                "default-features": "default" in enabled}
        if enabled - {"default"}:
            spec["features"] = sorted(enabled - {"default"})
        if alias in result:
            raise Refusal("E_FEATURES", "feature-unification alias collision")
        result[alias] = spec
    return result


def with_feature_unifiers(manifest: dict, targets: dict[str, dict[str, dict]]) -> dict:
    """One portable source manifest: exact native-target tables, original wasm/other-target specs unchanged."""
    result = copy.deepcopy(manifest)
    original_names = set(normal_dependencies(manifest))
    for target, dependencies in sorted(targets.items()):
        if any(name.startswith("__sunrise_sdk_features_") for name in original_names):
            raise Refusal("E_FEATURES", "a source dependency uses a reserved SDK feature-unification alias")
        entries = result.setdefault("target", {}).setdefault(target, {}).setdefault("dependencies", {})
        entries.update(dependencies)
    return result


def sdk_legs() -> list[tuple[str, bool]]:
    """The release's one authoritative packing list. `sdk-archives` is a read-only listing subcommand."""
    names = run([sys.executable, str(ROOT / "tools/ci/nightly.py"), "sdk-archives"], cwd=ROOT).splitlines()
    legs = []
    for name in names:
        if name == "api-source.tar.xz":
            continue
        if name.startswith("api-") and name.endswith(".tar.xz"):
            target = name[len("api-"):-len(".tar.xz")]
            server = target.endswith("-server")
            legs.append((target.removesuffix("-server") if server else target, server))
    if not legs or len(set(legs)) != len(legs):
        raise Refusal("E_FEATURES", "the release has no unique native SDK packing legs")
    return sorted(legs)


def game_leg(folder: Path, target: str, server: bool) -> tuple[dict, str]:
    """Resolve a published leg, without compiling or installing a cross toolchain/standard library."""
    cfg = set(run(["rustc", "--print", "cfg", "--target", target]).splitlines())
    flags = prebuilt_game.cargo_features(server, "linux" if 'target_os="linux"' in cfg else "")
    packages = ["-p", prebuilt_game.GAME]
    if not server and (folder / "crates" / prebuilt_game.OFFICIAL / "Cargo.toml").is_file():
        packages += ["-p", prebuilt_game.OFFICIAL]
    metadata = json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform",
                               target, *flags], cwd=folder))
    resolved = run(["cargo", "tree", "--locked", "--offline", *packages, "-e", "normal", "--target", target,
                    "--no-dedupe", "--prefix", "depth", "-f", "{p}|{f}", *flags], cwd=folder)
    return metadata, resolved


def native_feature_unifiers(folder: Path, legs: list[tuple[str, bool]]) -> dict[str, dict[str, dict[str, dict]]]:
    """Derive every supported native leg, so Linux-produced api-source is also Windows/macOS source.
    Client/server legs of the same target must agree; widening one to the other's closure is refused."""
    result = {root: {} for root in (API, WASMTIME)}
    for target, server in legs:
        metadata, tree_text = game_leg(folder, target, server)
        features, dependencies = target_build(tree_text, complete=True)
        for root in result:
            pins = feature_unifiers(metadata, root, features, dependencies)
            if target in result[root] and result[root][target] != pins:
                raise Refusal("E_FEATURES", f"{root}'s client/server feature closures differ for {target}")
            result[root][target] = pins
    return result


def target_metadata(folder: Path, environment: dict | None = None, *, offline: bool = False) -> dict:
    return json.loads(run(["cargo", "metadata", "--locked", *(["--offline"] if offline else []),
                          "--format-version", "1", "--filter-platform", prebuilt_game.rustc_host()],
                         cwd=folder, env=environment))


def tree(folder: Path, package: str, environment: dict | None = None) -> str:
    return run(["cargo", "tree", "--locked", "--offline", "-p", package, "-e", "normal", "--target",
                prebuilt_game.rustc_host(), "--no-dedupe", "--prefix", "depth", "-f", "{p}|{f}"], cwd=folder, env=environment)


def dep_info(path: Path) -> list[str]:
    """The files rustc read, from the first rule of a Makefile-style dep-info file (`\\ ` escapes a space)."""
    rule = path.read_text().split("\n", 1)[0]
    _, _, files = rule.partition(": ")
    return [file.replace("\x00", " ") for file in files.replace("\\ ", "\x00").split()]


def root_forms(roots: list[Path], crate: Crate) -> list[str]:
    """How the in-tree build script names the checkout: through the crate (`<root>/<crate>/../..`) or directly."""
    forms = {f"{root}/{crate.directory}/../.." for root in roots} | {str(root) for root in roots}
    return sorted(forms, key=len, reverse=True)


def relocate(code: str, forms: list[str]) -> str:
    for form in forms:
        code = code.replace(form + "/", ROOT_TOKEN + "/")
    return code


def check_files(files: dict[str, bytes], crates: list[Crate], tracked: set[str], forbidden: list[str]) -> list[str]:
    """The packing list: every path relative and in its place, every source file tracked by git, no file naming
    a local folder. Sorted, so the same files always give the same list."""
    directories = {crate.directory: crate for crate in crates}
    for path, data in files.items():
        if path.startswith("/") or "\\" in path or ".." in path.split("/"):
            raise Refusal("E_PATH", f"{path} is not a plain relative path")
        if path not in ("Cargo.toml", "Cargo.lock"):
            crate = next((crate for directory, crate in directories.items()
                          if path.startswith(directory + "/")), None)
            rest = path[len(crate.directory) + 1:] if crate else ""
            allowed = crate is not None and (
                rest == "Cargo.toml"
                or (crate.generated and (rest == "build.rs" or re.fullmatch(rf"{GENERATED}/[\w.-]+\.rs", rest)))
                or (not rest.startswith(GENERATED + "/") and path in tracked))
            if not allowed:
                raise Refusal("E_STRAY_FILE", f"{path} is no part of the API source")
        for folder in forbidden:
            # The folder itself or a path under it, not any text that starts the same way.
            if re.search(re.escape(folder.encode()) + rb'(?=[/"\\]|$)', data, re.MULTILINE):
                raise Refusal("E_ABSOLUTE_PATH", f"{path} names {folder}")
    return sorted(files)


def packing_index(files: dict[str, bytes]) -> str:
    return "".join(f"{hashlib.sha256(files[path]).hexdigest()}  {path}\n" for path in sorted(files))


def feature_disagreements(game: dict[str, set[str]], ours: dict[str, set[str]], packages: set[str]) -> list[str]:
    """The packages, among `packages`, whose resolved features differ between the game's build and ours. The name
    `default` itself is left out: what it turns on is in the resolved set, and a transitive package (memchr under
    serde_json) can get it from another crate of the game that the API source does not have."""
    def enabled(features: dict[str, set[str]], package: str) -> list[str] | None:
        return sorted(features[package] - {"default"}) if package in features else None

    return [f"{package}: game {enabled(game, package)}, ours {enabled(ours, package)}"
            for package in sorted(packages) if enabled(game, package) != enabled(ours, package)]


def lock_disagreements(slice_lock: dict, game_lock: dict) -> list[str]:
    """Packages the API source pins differently from the game's lockfile."""
    game = {(package["name"], package["version"], package.get("source"), package.get("checksum"))
            for package in game_lock["package"]}
    return sorted(f"{package['name']} {package['version']}" for package in slice_lock["package"]
                  if (package["name"], package["version"], package.get("source"), package.get("checksum")) not in game)


def build_messages(arguments: list[str], target_dir: Path, packages: tuple[str, ...] = (API, WASMTIME)) -> list[dict]:
    output = run(["cargo", "check", "--locked", *(f"-p{package}" for package in packages), "--message-format", "json",
                  *arguments], cwd=ROOT,
                 env={**os.environ, "CARGO_TARGET_DIR": str(target_dir)})
    return [json.loads(line) for line in output.splitlines() if line.startswith("{")]


def collect(crates: list[Crate], features: dict[str, dict], workspace_root: Path) -> dict[str, bytes]:
    """The generated files, the same from every build, and every file rustc read for the crates' libraries, over
    the host, wasm32 and all kept features. A read outside its crate is refused."""
    by_directory = {(workspace_root / crate.directory).resolve(): crate for crate in crates}
    every = [f"{crate.name}/{feature}" for crate in crates for feature in sorted(features[crate.name])]
    builds = {"host": [], WASM: ["--target", WASM], "host, all features": ["--features", ",".join(every)]}
    # The game's wasmtime is for native mods: it is read on the host, never built for wasm32.
    only = {WASM: (API,)}
    files: dict[str, bytes] = {}
    generated: dict[str, tuple[str, dict[str, bytes]]] = {}
    # A target folder of its own, fresh: no build script output folder holds a file an older generator wrote.
    target_root = Path(os.environ.get("CARGO_TARGET_DIR", workspace_root / "target"))
    target_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=target_root, prefix="sdk-api-source-build.") as target_dir:
        for label, arguments in builds.items():
            collect_build(build_messages(arguments, Path(target_dir), only.get(label, (API, WASMTIME))), label, by_directory, workspace_root,
                          files, generated)
    return files


def collect_build(messages: list[dict], label: str, by_directory: dict[Path, Crate], workspace_root: Path,
                  files: dict[str, bytes], generated: dict[str, tuple[str, dict[str, bytes]]]) -> None:
    """One build's generated files and reads, added to `files`; `generated` keeps the first build's outputs."""
    out_dirs = {}
    for message in messages:
        if message["reason"] != "build-script-executed" or not message["package_id"].startswith("path+"):
            continue
        if any(message[key] for key in ("cfgs", "env", "linked_libs", "linked_paths")):
            raise Refusal("E_BUILD_SCRIPT", f"{message['package_id']} sets more than generated files")
        out_dirs[message["package_id"]] = Path(message["out_dir"])
    for message in messages:
        if message["reason"] != "compiler-artifact" or "custom-build" in message["target"]["kind"]:
            continue
        crate = by_directory.get(Path(message["manifest_path"]).resolve().parent)
        if crate is None:
            continue
        out_dir = out_dirs.get(message["package_id"])
        if crate.generated:
            stray = [path.name for path in out_dir.iterdir()
                     if not path.is_file() or not re.fullmatch(r"[\w.-]+\.rs", path.name)]
            if stray:
                raise Refusal("E_GENERATED_ENTRY", f"{crate.name}'s build script wrote {', '.join(sorted(stray))}, "
                                                   "not a Rust file")
            outputs = {path.name: relocate(path.read_text(), root_forms([workspace_root, ROOT], crate)).encode()
                       for path in sorted(out_dir.iterdir())}
            first = generated.setdefault(crate.name, (label, outputs))
            if first[1] != outputs:
                differing = sorted(name for name in first[1].keys() | outputs.keys()
                                   if first[1].get(name) != outputs.get(name))
                raise Refusal("E_TARGET_DEPENDENT", f"{crate.name}: {', '.join(differing)} differ between "
                                                    f"{first[0]} and {label}")
            for name, data in outputs.items():
                files[f"{crate.directory}/{GENERATED}/{name}"] = data
        artifact = Path(message["filenames"][0])
        digest = artifact.name.split(".")[0].rsplit("-", 1)[1]
        info = artifact.parent / f"{message['target']['name'].replace('-', '_')}-{digest}.d"
        for read in dep_info(info):
            path = Path(os.path.normpath(workspace_root / read))
            if out_dir is not None and path.parent == out_dir:
                continue
            if not path.is_relative_to(workspace_root / crate.directory):
                raise Refusal("E_SOURCE_OUTSIDE_CRATE", f"{crate.name} reads {path}")
            files[path.relative_to(workspace_root).as_posix()] = path.read_bytes()
    print(f"{label}: {len(files)} files so far", flush=True)


def build(out: Path) -> dict[str, bytes]:
    """Write the API source into OUT, which must be missing or empty. Everything is checked in a folder beside
    it first: a refused build leaves OUT as it was."""
    if out.exists() and any(out.iterdir()):
        raise Refusal("E_OUT_NOT_EMPTY", f"{out} is not empty")
    metadata = json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
    workspace_root = Path(metadata["workspace_root"])
    crates = closure(metadata, (API, WASMTIME))
    by_name = {crate.name: crate for crate in crates}
    workspace = tomllib.loads((workspace_root / "Cargo.toml").read_text())
    manifests = {crate.name: tomllib.loads((workspace_root / crate.directory / "Cargo.toml").read_text())
                 for crate in crates}
    features = kept_features(manifests)
    native = native_feature_unifiers(workspace_root, sdk_legs())
    files = collect(crates, features, workspace_root)
    tracked = set(filter(None, run(["git", "ls-files", "-z"], cwd=workspace_root).split("\0")))
    # The whole source tree, so no file a build on another host or profile reads is left behind: the commit's own
    # bytes, the same on every OS whatever line endings the working tree has.
    commit = run(["git", "rev-parse", "HEAD"], cwd=workspace_root).strip()
    sources = tuple(f"{crate.directory}/src" for crate in crates)
    files.update(prebuilt_game.tracked_blobs(workspace_root, commit, sources))
    for crate in crates:
        files[f"{crate.directory}/Cargo.toml"] = normalise_manifest(
            with_feature_unifiers(manifests[crate.name], native.get(crate.name, {})),
            workspace, crate, by_name, features[crate.name]).encode()
        if crate.generated:
            files[f"{crate.directory}/build.rs"] = BUILD_SCRIPT.encode()
    resolver = workspace.get("workspace", {}).get("resolver", "2")
    files["Cargo.toml"] = ("# The mod API's source with its generated code, for one game build.\n"
                           + table("workspace", {"resolver": resolver,
                                                 "members": [crate.directory for crate in crates]})).encode()
    forbidden = forbidden_folders(workspace_root)
    check_files(files, crates, tracked, forbidden)
    out.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=out.parent, prefix=f".{out.name}.") as staging:
        staged = Path(staging) / "api"
        for path, data in files.items():
            (staged / path).parent.mkdir(parents=True, exist_ok=True)
            (staged / path).write_bytes(data)
        # Cargo keeps the game's pins and drops the packages the API does not use.
        shutil.copyfile(workspace_root / "Cargo.lock", staged / "Cargo.lock")
        run(["cargo", "metadata", "--format-version", "1"], cwd=staged, env=isolated())
        files["Cargo.lock"] = (staged / "Cargo.lock").read_bytes()
        disagreeing = lock_disagreements(tomllib.loads(files["Cargo.lock"].decode()),
                                         tomllib.loads((workspace_root / "Cargo.lock").read_text()))
        if disagreeing:
            raise Refusal("E_LOCK", "pinned differently from the game: " + ", ".join(disagreeing))
        check_files(files, crates, tracked, forbidden)
        if out.exists():
            out.rmdir()
        staged.rename(out)
    print(f"{len(files)} files, {sum(map(len, files.values()))} bytes; crates: "
          + ", ".join(crate.name for crate in crates), flush=True)
    return files


def forbidden_folders(workspace_root: Path) -> list[str]:
    """The checkout, its target folder and Cargo's home: no file of the API source may name a path under them."""
    folders = {workspace_root, ROOT, Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")),
               Path(os.environ.get("CARGO_TARGET_DIR", workspace_root / "target"))}
    return sorted(str(folder) for folder in folders if len(folder.parts) > 1)


def isolated() -> dict[str, str]:
    """Cargo's environment for the API source: never the checkout's target folder."""
    return {key: value for key, value in os.environ.items() if key != "CARGO_TARGET_DIR"}


def archive_sizes(folder: Path) -> dict[str, int]:
    """Raw, gzip and xz sizes of a reproducible tar of the folder (sorted names, no owners, time zero)."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for path in sorted(p for p in folder.rglob("*") if p.is_file()):
            info = tarfile.TarInfo(path.relative_to(folder).as_posix())
            info.size, info.mode, info.mtime = path.stat().st_size, 0o644, 0
            archive.addfile(info, io.BytesIO(path.read_bytes()))
    raw = buffer.getvalue()
    return {"files": sum(p.stat().st_size for p in folder.rglob("*") if p.is_file()), "tar": len(raw),
            "gz": len(gzip.compress(raw, 9, mtime=0)), "xz": len(lzma.compress(raw, preset=9 | lzma.PRESET_EXTREME))}


def starter_mod(folder: Path, wasmtime: bool = False) -> None:
    """The native template as `cargo sunrise-mod new` writes it, with the API dependency in the path form. With
    `wasmtime`, the native consumer of the game's wasmtime: it also names `sunrise-wasmtime`, as a mod with a
    `[[native.*]]` row may."""
    name = "starter-wasmtime" if wasmtime else "starter"

    def fill(text: str) -> str:
        # A native mod's manifest carries no SDK metadata block (cargo-sunrise-mod's `new`).
        return text.replace("{id}", name).replace("{name}", "Starter").replace("{metadata}", "")

    manifest, count = re.subn(r"^sunrise-mod-api = .*$", f'sunrise-mod-api = {{ path = "{MOD_PATH}/crates/{API}" }}',
                              fill((TEMPLATE / "api.toml").read_text()), flags=re.MULTILINE)
    if count != 1:
        raise Refusal("E_TEMPLATE", "the template names sunrise-mod-api once")
    # Its own workspace: cargo must not climb to a folder above it.
    (folder / "src").mkdir(parents=True)
    if wasmtime:
        manifest += f'sunrise-wasmtime = {{ path = "{MOD_PATH}/crates/{WASMTIME}" }}\n'
    (folder / "Cargo.toml").write_text(manifest + "\n[workspace]\n")
    (folder / "src/lib.rs").write_text(fill((TEMPLATE / "lib.rs").read_text()))


def check_root(game: dict, ours: dict, root: str, game_tree: str, consumer_tree: str, consumer: str,
               exact: bool = True) -> int:
    """One API root's closure and features: the prebuilt game's against the API source's, and `consumer`'s own
    resolution (its `cargo tree`) against the game's. Each root is judged by a consumer that depends on it: a starter
    that names only `sunrise-mod-api` says nothing of `sunrise-wasmtime`. Returns the closure's size. A root that is
    not `exact` may resolve fewer packages and features than the game: the game's other crates turn on features of
    wasmtime's dependencies (rustls and ring turn on `digest`'s `subtle` and `rand_core`), which the API source does
    not have; it must still resolve nothing, and turn on no feature, that the game does not. The root and `wasmtime` themselves
    are exact: a mod's local check sees the surface the launcher binds."""

    game_features, game_dependencies = target_build(game_tree, complete=True)
    own_features, own_dependencies = target_build(consumer_tree, complete=True)

    def closure(metadata: dict, features: dict[str, set[str]],
                dependencies: dict[str, set[str]]) -> set[tuple[str, str]]:
        packages = {package["id"]: package for package in metadata["packages"]}
        return {(packages[package]["name"], packages[package]["version"])
                for package in resolved_runtime_packages(metadata, root, features, dependencies)}

    game_closure = closure(game, game_features, game_dependencies)
    own_closure = closure(ours, own_features, own_dependencies)
    if (game_closure != own_closure) if exact else not own_closure <= game_closure:
        raise Refusal("E_CLOSURE", f"the prebuilt game's {root} closure {sorted(game_closure - own_closure)} "
                                   f"against the API source's {sorted(own_closure - game_closure)}")
    if exact:
        differing = feature_disagreements(game_features, own_features, {f"{n} {v}" for n, v in own_closure})
    else:
        # The consumer turns on no feature the game does not: the game's others may add features, never the reverse.
        differing = [f"{key}: game {sorted(game_features.get(key, set()) - {'default'})}, "
                     f"ours {sorted(features - {'default'})}"
                     for key, features in sorted(own_features.items())
                     if key in game_features and features - game_features[key]]
        # The root itself and wasmtime are the surface a mod names: exactly the game's, as the launcher binds them.
        named = {key for key in game_features.keys() | own_features.keys() if key.split(" ")[0] in (root, "wasmtime")}
        differing += [line for line in feature_disagreements(game_features, own_features, named)
                      if line not in differing]
    if differing:
        raise Refusal("E_FEATURES", f"{consumer} compiles {root} with other features than the game: "
                      + "; ".join(differing))
    return len(own_closure)


def check(api: Path, scratch: Path) -> None:
    """The starter mod checks against the API source alone, for the host and wasm32, offline after one fetch; the
    API source's crates are the prebuilt game's API closure at the game's versions."""
    if scratch.exists() and any(scratch.iterdir()):
        if not (scratch / SCRATCH_MARK).is_file():
            raise Refusal("E_SCRATCH", f"{scratch} is not empty and not an earlier check's folder")
        shutil.rmtree(scratch)
    scratch.mkdir(parents=True, exist_ok=True)
    (scratch / SCRATCH_MARK).write_text("")
    mod = scratch / "starter"
    starter_mod(mod)
    native = scratch / "starter-wasmtime"
    starter_mod(native, wasmtime=True)
    for folder in (mod, native):
        shutil.copytree(api, folder / MOD_PATH)
        shutil.copyfile(api / "Cargo.lock", folder / "Cargo.lock")
    environment = {**isolated(), "CARGO_TARGET_DIR": str(scratch / "target"), "RUSTFLAGS": "-D warnings"}
    # Each is the root of its own workspace: cargo climbs to no folder above it.
    for folder, expected in ((mod, mod / "Cargo.toml"),
                             (mod / MOD_PATH / "crates" / API, mod / MOD_PATH / "Cargo.toml")):
        found = json.loads(run(["cargo", "locate-project", "--workspace"], cwd=folder, env=environment))["root"]
        if Path(found) != expected:
            raise Refusal("E_WORKSPACE_ESCAPE", f"{folder} belongs to {found}, not {expected}")
    run(["cargo", "fetch"], cwd=mod, env=environment)
    run(["cargo", "fetch"], cwd=native, env=environment)
    timings = {}
    for label, command, folder in (
            ("starter, host", ["cargo", "check", "--offline"], mod),
            ("starter, wasm32", ["cargo", "check", "--offline", "--target", WASM], mod),
            # The game's wasmtime is for native mods: host only, never wasm32.
            ("native starter with sunrise-wasmtime, host", ["cargo", "check", "--offline"], native),
            ("API source, host, all features", ["cargo", "check", "--offline", "--locked", "--workspace",
                                                "--all-features"], mod / MOD_PATH)):
        started = time.monotonic()
        run(command, cwd=folder, env=environment)
        timings[label] = time.monotonic() - started
    game, game_tree = game_leg(ROOT, prebuilt_game.rustc_host(), False)
    ours = target_metadata(mod / MOD_PATH, environment, offline=True)

    sizes = {root: check_root(game, ours, root, game_tree, tree(folder, name, environment), name, exact)
             for root, folder, name, exact in ((API, mod, "mod-starter", True),
                                               (WASMTIME, native, "mod-starter-wasmtime", True))}
    for folder, name in ((mod, "mod-starter"), (native, "mod-starter-wasmtime")):
        disagreeing = lock_disagreements(tomllib.loads((folder / "Cargo.lock").read_text()),
                                         tomllib.loads((ROOT / "Cargo.lock").read_text()))
        if disagreeing != [f"{name} 0.1.0"]:
            raise Refusal("E_LOCK", f"{name} resolves differently from the game: " + ", ".join(disagreeing))
    print("closures, as the prebuilt game's, with its features and versions: "
          + ", ".join(f"{root} {size} crates" for root, size in sizes.items()))
    for label, seconds in timings.items():
        print(f"cargo check, {label}: {seconds:.1f}s")
    print("size: " + ", ".join(f"{kind} {size}" for kind, size in archive_sizes(api).items()))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("build").add_argument("out", type=Path)
    archive_parser = commands.add_parser("archive")
    archive_parser.add_argument("out", type=Path)
    archive_parser.add_argument("archive", type=Path)
    check_parser = commands.add_parser("check")
    check_parser.add_argument("out", type=Path)
    check_parser.add_argument("--scratch", type=Path, help="default: the target folder's sdk-api-source-check")
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(args.out.resolve())
        elif args.command == "archive":
            # The package's own archive form: a mod's `.sunrise/sdk/api` is this folder unpacked.
            prebuilt_game.tar_xz(args.out, prebuilt_game.tree(args.out), args.archive)
        else:
            target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
            check(args.out.resolve(), (args.scratch or target / "sdk-api-source-check").resolve())
    except Refusal as refusal:
        sys.exit(f"error: {refusal}")


if __name__ == "__main__":
    main()
