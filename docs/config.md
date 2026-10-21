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

The run stops with exit code `1` when the configuration file cannot be used.
