# Host a modded server

Use `sunrise-server` on Linux, macOS or Windows. A headless Linux server needs no desktop, graphics or Minecraft account. See [Hosting](hosting.md) for machine requirements and networking.

## 1. Install

```sh
curl -fsSL https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.sh | sh
```

On Windows, run `irm https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.ps1 | iex` in PowerShell. The installer verifies the release and does not need administrator access.

## 2. Create the server with its mods

```sh
mkdir smp && cd smp
sunrise-server init --eula --mods coords_hud,waystones
```

Choose IDs from `sunrise-server mods search TEXT` in a server folder, or use `--pack ID` with `init` to start from a registry pack. `--eula` means you accept the [Minecraft EULA](https://aka.ms/MinecraftEULA). See [Prepare the game files](hosting.md#2-prepare-the-game-files) for what the engine needs; mod packages do not contain Minecraft files.

`init` installs the selected mods and their dependencies, and pins the versions and checksums in `mods.lock`. It leaves client-only pack members out of the server. Check what was selected with `sunrise-server mods list`, or add `--json` for the lock as JSON.

**Native mods run unsandboxed with the server account's permissions.** Only allow code from authors and sources you trust. The installer lists those sources and asks before downloading native code. In a script, `--trust-native-code` explicitly permits it. Without permission, unattended `init` writes the configuration and independent content mods, then prints the command to finish installing the native mods.

Native mods require an SDK with a server build for your platform. An exact signed prebuilt mod is used when available; otherwise the server downloads the SDK's pinned Rust compiler to its own data folder and builds the mod. Source builds on Linux also need a C linker (`build-essential` on Debian and Ubuntu, `gcc` on Fedora). See [Native server export](hosting.md#native-server-export) for using a manually built server instead of registry management.

## 3. Start it

```sh
sunrise-server start
sunrise-server status
```

Players join at the server's address, on port 25565 by default. To run at boot, use [`sunrise-server service install`](hosting.md#8-start-it-with-the-computer). For containers, see [Docker](hosting.md#9-docker-build-it-yourself).

## 4. Give players the same mods

```sh
sunrise-server mods client-pack --out smp.spack
```

Send players `smp.spack`. It lists the exact versions and checksums of the enabled packages that both sides need. It contains no server-only mods, mod binaries, configuration overrides or world data. Players can use those pins to set up a matching profile. A player does not need mods that are only installed on the server.

## 5. Change mods later

```sh
sunrise-server stop
sunrise-server mods add better_paths
sunrise-server mods disable waystones
sunrise-server mods update --check   # preview without applying
sunrise-server mods update           # review and apply
sunrise-server mods rollback         # undo the last mod change
sunrise-server start
```

`disable` keeps the package installed; `enable` turns it back on. Remove a direct request with `mods remove ID`. A change that leaves another mod without its required dependency is refused. If the registry takes a mod down, Sunrise warns but does not disable it for you.

`sunrise-server update` stages the matching mods before switching the engine. If any mod does not fit, it leaves the installed release and mod selection in place. `sunrise-server rollback` restores the previous release and its saved mod selection. `start` and `run` never change your mods: if the files, selection or engine disagree with the lock, they stop and print the command to put things right.

To move the server, copy its entire folder, including `server.toml`, `mods.lock`, and the world. Install the matching Sunrise release and make its prepared game files available on the new machine. Native runtime files are specific to the installed build and the platform.
