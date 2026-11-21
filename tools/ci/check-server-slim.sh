#!/usr/bin/env bash
# Check the unpacked Linux server archive on Debian's slim root, without Docker,
# installed packages or Minecraft files. Do not let the test see the host sysroot.
set -euo pipefail

archive=${1:?pass the path to the server tar.gz}
build=${2:?pass the full commit of the server programs}
[[ $build =~ ^[0-9a-f]{40}$ ]]
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/sunrise-server-slim.XXXXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir "$work/root" "$work/archive"

registry=https://registry-1.docker.io/v2/library/debian
curl_args=(--fail --silent --show-error --location --proto '=https' --proto-redir '=https' --retry 2 --max-time 180)
token=$(curl "${curl_args[@]}" 'https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/debian:pull' | jq -er '.token')
fetch() { curl "${curl_args[@]}" -H "Authorization: Bearer $token" "$@"; }
verify() {
    local hash=$1 path=$2
    [[ $hash =~ ^sha256:[0-9a-f]{64}$ ]]
    printf '%s  %s\n' "${hash#sha256:}" "$path" | sha256sum --check --status
}

digest=$(fetch -H 'Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json' \
    "$registry/manifests/stable-slim" | jq -er '[.manifests[] | select(.platform.os == "linux" and .platform.architecture == "amd64")][0].digest')
[[ $digest =~ ^sha256:[0-9a-f]{64}$ ]]
fetch -H 'Accept: application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json' \
    "$registry/manifests/$digest" > "$work/manifest.json"
verify "$digest" "$work/manifest.json"
# Debian slim is a single root filesystem layer; don't treat an OCI overlay like an ordinary tar.
layer=$(jq -er '.layers | select(length == 1) | .[0].digest' "$work/manifest.json")
[[ $layer =~ ^sha256:[0-9a-f]{64}$ ]]
fetch "$registry/blobs/$layer" > "$work/root.tar.gz"
verify "$layer" "$work/root.tar.gz"
tar --no-same-owner --no-same-permissions -xzf "$work/root.tar.gz" -C "$work/root"

tar -xzf "$archive" -C "$work/archive"
shopt -s nullglob
folders=("$work/archive"/sunrise-server-*)
[[ ${#folders[@]} -eq 1 ]]
install -Dm755 "${folders[0]}/sunrise-engine" "$work/root/sunrise/sunrise-engine"
install -Dm755 "${folders[0]}/sunrise-server" "$work/root/sunrise/sunrise-server"
# The Rust guest stack needs /proc/self/exe. Mount procfs only for this PID namespace.
run() {
    unshare --user --map-root-user --mount --pid --fork --mount-proc="$work/root/proc" \
        chroot "$work/root" /usr/bin/env -i HOME=/tmp PATH=/usr/bin:/bin "$@"
}
run /sunrise/sunrise-server --help
run /sunrise/sunrise-server init --help
help=$(run /sunrise/sunrise-engine --help)
# Stable dispatches can package a source ref older than this workflow's build-id option.
if [[ $help == *--build-id* ]]; then
    actual=$(run /sunrise/sunrise-engine --build-id)
    [[ $actual == "$build" ]] || { echo "Expected engine build $build, got: $actual" >&2; exit 1; }
    printf 'Server archive starts on debian:stable-slim (%s), engine %s\n' "$digest" "$actual"
else
    [[ $help == *--resources* ]]
    printf 'Server archive starts on debian:stable-slim (%s); this engine does not report a build ID\n' "$digest"
fi
