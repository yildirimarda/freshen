# Configuration file

freshen reads an optional TOML file from the location that is conventional
for the operating system it runs on:

| Platform | Location |
|---|---|
| macOS, Linux | `$XDG_CONFIG_HOME/freshen/config.toml`, else `~/.config/freshen/config.toml` |
| Windows | `%APPDATA%\freshen\config.toml` |

The file is optional. When it is missing, freshen runs with its built-in
defaults; when it is present, every section and every key in it is optional
too, so a file that sets a single value is valid.

```toml
# ~/.config/freshen/config.toml

[preflight]
network = false
min_free_disk_gib = 5

[run]
jobs = 8

[steps]
brew-casks = false

[exclude]
casks = ["dropbox"]

[apps]
never_quit = ["com.docker"]
```

A different file can be used with `freshen --config /path/to/config.toml`. A
file named that way has to exist; the file in the default location does not.

## Schema

### `[preflight]`

The checks that run before an update starts.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `network` | bool | `true` | Check that the network is reachable. |
| `disk_space` | bool | `true` | Check that enough disk space is free. |
| `battery` | bool | `true` | Check that the battery is not too low. |
| `metered_connection` | bool | `true` | Check that the connection is not metered or a hotspot. |
| `min_free_disk_gib` | integer | `1` | Free disk space in GiB that an update needs. |
| `battery_threshold` | integer | `20` | Battery percentage below which an update on battery is refused. At most `100`. |

### `[package_manager]`

What happens when another package manager is already running.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `true` | Wait for another package manager instead of running at the same time. |
| `wait_seconds` | integer | `30` | Seconds to wait for the other package manager before aborting. |

### `[timeouts]`

How long a step and the whole run may take. `0` disables the timeout.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `step_seconds` | integer | `0` | Seconds a single step may take. |
| `total_seconds` | integer | `0` | Seconds the whole run may take. |

### `[run]`

How the run itself behaves.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `jobs` | integer | `4` | How many read-only `--check` queries run at the same time. At least `1`. |
| `snapshot_retention` | integer | `10` | How many pre-upgrade snapshot files to keep. `0` keeps every snapshot. |

### `[steps]`

Which steps may run. Every key is a step id; `false` disables the step and
`true` enables it. A step that is not named stays enabled.

```toml
[steps]
brew-casks = false
brew-autoremove = false
docker = true
```

A disabled step is reported as skipped with the reason "disabled in the
configuration file", in the run summary and in `--list-steps`. A step id that
is not registered is an error, so a typo does not silently do nothing.

Disabling a step also skips any steps that declare it as a prerequisite,
because they need the disabled step to run.

`freshen --list-steps` names every step, so the ids to use here are the ones
in that listing.

### `[include]` and `[exclude]`

Which items a run may touch. Both sections take the same four keys, and the
exclude list always wins over the include list.

| Key | Type | Meaning |
|---|---|---|
| `casks` | list of names | Homebrew casks. |
| `packages` | list of names | Packages: Homebrew formulae and uv tools. |
| `containers` | list of names | Standalone containers, by container name or container id. |
| `projects` | list of names | Compose projects, by project name. |
| `apps` | list of App Store ids | Mac App Store apps, matched by their App Store id (`mas upgrade <id>`). |

An empty include list means "everything of this kind", so a file with only an
exclude list keeps every other item in scope.

```toml
[include]
packages = ["node", "python"]

[exclude]
casks = ["dropbox"]
projects = ["legacy-stack"]
```

The lists are applied to the items a run upgrades, not to the whole tool:

* a cask that is not in scope is not upgraded, and it is not reported as
  outdated by `--check`,
* a package that is not in scope is not upgraded, and it is not reported as
  outdated by `--check`,
* a compose project that is not in scope is not pulled, not brought up and not
  stopped when the container runtime is upgraded,
* a container that is not in scope is not stopped when the container runtime is
  upgraded, and it is therefore not started again afterwards,
* a Mac App Store app that is not in scope is not upgraded, and it is not
  reported as outdated by `--check`.

Names are matched exactly, the way the package managers spell them.

### `[apps]`

Which applications are quit around a cask upgrade.

| Key | Type | Meaning |
|---|---|---|
| `never_quit` | list of names | Applications that are never quit. |
| `always_quit` | list of names | Applications that are always quit, without asking. |

```toml
[apps]
never_quit = ["com.docker", "Terminal.app"]
always_quit = ["com.spotify"]
```

An entry names an application by its bundle identifier (`com.docker`) or by
its name (`Docker` or `Docker.app`). The comparison ignores case and the
`.app` suffix.

* An application on the never-quit list is left running, exactly like the
  terminal that runs freshen. Its cask is still upgraded.
* An application on the always-quit list is quit without asking. When every
  application that would be quit is on that list, the confirmation prompt is
  not shown at all.

The `NEVER_QUIT` and `ALWAYS_QUIT` environment variables hold the same lists
as a comma-separated value. They add to the lists in the configuration file
instead of replacing them:

```bash
NEVER_QUIT="com.docker,com.orbstack" freshen
```

## Precedence

1. A flag given on the command line wins.
2. The configuration file wins over the built-in default.
3. The built-in default is used when neither says anything.

```bash
# The file asks for 1 GiB of free disk space, the command line for 20 GiB.
freshen --min-free-disk-gib 20
```

The flags that ask for confirmation (`--yes`), that kill applications
(`--force-kill`) and that prune caches and images (`--cleanup`) are not
configurable: they are decided on the command line, every time.

## Validation

An unknown key, a value of the wrong type and a value outside its range are
refused before anything is changed. The error names the file and points at
the line that has to be fixed:

```
freshen: /home/user/.config/freshen/config.toml:3:1: unknown field `netwrok`, expected one of `network`, `disk_space`, `battery`, `metered_connection`, `min_free_disk_gib`, `battery_threshold`
freshen: /home/user/.config/freshen/config.toml:4:8: invalid type: string "many", expected usize
freshen: /home/user/.config/freshen/config.toml:2:21: battery_threshold must be between 0 and 100, got 200
```

A file with a syntax error is refused the same way, with the position of the
syntax error:

```
freshen: /home/user/.config/freshen/config.toml:1:11: unclosed table, expected `]`
```

A step id in the `[steps]` section that is not registered is refused as well,
with the known step ids:

```
freshen: unknown step `brew-cask` in the configuration file (known steps: brew, brew-autoremove, brew-bundle-cleanup, brew-casks, brew-cleanup, brew-doctor, brew-formulae, brew-reopen, brew-services, colima-start, colima-stop, docker, macos-updates, mas, mise, uv)
```

The container steps (`colima-stop`, `colima-start`, `docker`) work with any
supported runtime: Colima (including Lima VMs), OrbStack, Docker Desktop,
Rancher Desktop, Podman (machine) and native Docker Engine. The runtime is
detected from `DOCKER_HOST`, `docker context show` and well-known socket
paths; override it with `--container-runtime` (`colima`, `orbstack`,
`docker-desktop`, `rancher-desktop`, `podman`, `docker`).

The run stops with exit code `1` when the configuration file cannot be used.
