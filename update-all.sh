#!/bin/zsh
# update-all.sh — bulk update for brew + uv + mise
# - Quits running apps that are about to be upgraded via cask, then reopens them afterwards.
# - If colima/lima is about to be upgraded and colima is running: brings running compose projects
#   down, stops standalone (docker run) containers, stops colima, upgrades, starts colima and
#   brings everything back up.
# - Pulls newer images for running compose projects and recreates only the changed containers.
#
# Environment variables (optional):
#   FORCE_KILL=1     Force-kill an app if it does not quit gracefully (unsaved data may be lost)
#   QUIT_TIMEOUT=20  Seconds to wait for an app to quit
#   ASSUME_YES=1     Don't ask before quitting apps for cask upgrades

setopt NO_UNSET 2>/dev/null
FORCE_KILL=${FORCE_KILL:-0}
QUIT_TIMEOUT=${QUIT_TIMEOUT:-20}
ASSUME_YES=${ASSUME_YES:-0}

# Terminal app running this script (Ghostty etc.) — so we never quit ourselves
SELF_BUNDLE_ID=${__CFBundleIdentifier:-}

reopen_ids=()
failed_steps=()
compose_projects=()        # entries: "<project name>\t<config files, comma separated>"
standalone_containers=()   # entries: "<container id>\t<name>"  (containers not managed by compose)
colima_was_running=0

log()  { printf '\n\033[1;34m==>\033[0m \033[1m%s\033[0m\n' "$1"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$1"; }

run() {
  # Run a command; on failure record it and keep going
  "$@" || { warn "Failed: $*"; failed_steps+=("$*"); }
}

bundle_id_of() {
  defaults read "$1/Contents/Info" CFBundleIdentifier 2>/dev/null
}

# Is the main executable of this .app bundle running? (ignores helper/agent processes
# that share the bundle id, which would make AppleScript's "is running" return true)
# NOTE: take the ps snapshot first, then filter it — a direct "ps | grep PATTERN" would
# always match grep's own command line.
is_running() {
  local procs
  procs="$(ps -axo command=)"
  print -r -- "$procs" | grep -F -q "$1/Contents/MacOS/"
}

pids_of_app() {
  local procs
  procs="$(ps -axo pid=,command=)"
  print -r -- "$procs" | grep -F "$1/Contents/MacOS/" | awk '{print $1}'
}

quit_app() {
  local app="$1" bid="$2" waited=0

  # with timeout: don't let osascript hang if the app shows a "Save changes?" dialog
  osascript \
    -e "with timeout of 3 seconds" \
    -e "tell application id \"$bid\" to quit" \
    -e "end timeout" >/dev/null 2>&1

  while is_running "$app" && (( waited < QUIT_TIMEOUT )); do
    sleep 1; (( waited++ ))
  done

  if is_running "$app"; then
    if [[ "$FORCE_KILL" == "1" ]]; then
      warn "$app did not quit, force-killing"
      pids_of_app "$app" | xargs kill -9 2>/dev/null
      sleep 2
    else
      warn "$app did not quit (use FORCE_KILL=1 to force). Upgrade will be attempted anyway."
      return 1
    fi
  fi
  return 0
}

# Print running compose projects as "<name>\t<config files>" lines
list_compose_projects() {
  docker compose ls --format json 2>/dev/null | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for p in data:
    name, files = p.get("Name"), p.get("ConfigFiles")
    if name and files:
        print(f"{name}\t{files}")
'
}

# Run "docker compose <args>" for a saved project entry
compose_cmd() {
  local entry="$1"; shift
  local name="${entry%%$'\t'*}" files="${entry#*$'\t'}"
  local args=(-p "$name")
  for f in ${(s:,:)files}; do args+=(-f "$f"); done
  run docker compose "${args[@]}" "$@"
}

# ---------------------------------------------------------------- Homebrew
if command -v brew >/dev/null 2>&1; then
  log "Homebrew: update"
  run brew update

  outdated_formulae=(${(f)"$(brew outdated --formula --quiet 2>/dev/null)"})

  # ---- zellij: server/client must match; sessions opened before the upgrade can't be attached from the new client
  if (( ${outdated_formulae[(Ie)zellij]} )) && [[ -n "${ZELLIJ:-}" ]]; then
    warn "zellij will be upgraded while you are inside a zellij session. Close and recreate your sessions afterwards."
  fi

  # ---- colima/lima: stop before upgrading (only if outdated AND colima is running)
  if (( ${outdated_formulae[(Ie)colima]} || ${outdated_formulae[(Ie)lima]} )) \
     && command -v colima >/dev/null 2>&1 && colima status >/dev/null 2>&1; then
    colima_was_running=1
    log "colima/lima is outdated and colima is running: stopping containers and colima"

    if command -v docker >/dev/null 2>&1; then
      # compose projects (name + config files)
      compose_projects=(${(f)"$(list_compose_projects)"})

      # standalone containers = running containers without a compose project label (docker run, k3d, ...)
      standalone_containers=(${(f)"$(docker ps --format $'{{.ID}}\t{{.Names}}\t{{.Label "com.docker.compose.project"}}' 2>/dev/null \
        | awk -F'\t' '$3 == "" { print $1 "\t" $2 }')"})

      for entry in "${compose_projects[@]}"; do
        echo "docker compose down: ${entry%%$'\t'*}"
        compose_cmd "$entry" down
      done

      for entry in "${standalone_containers[@]}"; do
        echo "docker stop: ${entry#*$'\t'}"
        run docker stop "${entry%%$'\t'*}" >/dev/null
      done
    fi

    run colima stop
  fi

  # Formulae first: this can take a while, no need to keep apps closed during it
  log "Homebrew: upgrade (formulae)"
  run brew upgrade --formula

  # ---- colima: start again and bring containers back up
  if (( colima_was_running )); then
    log "Starting colima and containers"
    if run colima start; then
      for entry in "${standalone_containers[@]}"; do
        echo "docker start: ${entry#*$'\t'}"
        run docker start "${entry%%$'\t'*}" >/dev/null
      done
      for entry in "${compose_projects[@]}"; do
        echo "docker compose up -d: ${entry%%$'\t'*}"
        compose_cmd "$entry" up -d
      done
    else
      warn "colima did not start; containers were NOT restarted"
    fi
  fi

  log "Homebrew: checking for outdated casks"
  casks=(${(f)"$(brew outdated --cask --greedy --quiet 2>/dev/null)"})

  if (( ${#casks} > 0 )); then
    echo "Casks to upgrade: ${casks[*]}"

    app_names=(${(f)"$(brew info --cask --json=v2 "${casks[@]}" 2>/dev/null | python3 -c '
import json, sys, os
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for c in data.get("casks", []):
    for a in c.get("artifacts", []):
        if not isinstance(a, dict):
            continue
        for x in a.get("app", []) or []:
            if isinstance(x, str):
                print(os.path.basename(x))
            elif isinstance(x, dict) and isinstance(x.get("target"), str):
                print(os.path.basename(x["target"]))
')"})

    # Collect running apps that would need to be quit
    to_quit_apps=(); to_quit_bids=()
    for name in "${app_names[@]}"; do
      for dir in /Applications "$HOME/Applications"; do
        app="$dir/$name"
        [[ -d "$app" ]] || continue
        bid="$(bundle_id_of "$app")"
        [[ -n "$bid" ]] || continue

        if [[ -n "$SELF_BUNDLE_ID" && "$bid" == "$SELF_BUNDLE_ID" ]]; then
          warn "$name is the terminal running this script, not quitting it (restart it manually after the upgrade)."
          continue
        fi

        if is_running "$app"; then
          to_quit_apps+=("$app"); to_quit_bids+=("$bid")
        fi
      done
    done

    do_casks=1; do_quit=1
    if (( ${#to_quit_apps} > 0 )); then
      echo "Running apps that will be quit and reopened:"
      printf '   - %s\n' "${to_quit_apps[@]:t}"
      if [[ "$ASSUME_YES" != "1" ]]; then
        read "ans?Quit these apps and upgrade casks? [Y]es / [s]kip quitting / [n]o: "
        case "${ans:l}" in
          ""|y|yes) ;;
          s|skip)   do_quit=0 ;;
          *)        do_casks=0 ;;
        esac
      fi
    fi

    if (( do_casks )); then
      if (( do_quit )); then
        for (( i = 1; i <= ${#to_quit_apps}; i++ )); do
          echo "Quitting: ${to_quit_apps[i]:t}"
          reopen_ids+=("${to_quit_bids[i]}")
          quit_app "${to_quit_apps[i]}" "${to_quit_bids[i]}"
        done
      fi

      log "Homebrew: upgrade (casks, --greedy)"
      run brew upgrade --cask --greedy
    else
      warn "Cask upgrade skipped by user."
    fi
  else
    echo "No outdated casks."
  fi

  log "Homebrew: autoremove"
  run brew autoremove

  log "Homebrew: cleanup"
  run brew cleanup --prune=all

  if (( ${#reopen_ids} > 0 )); then
    log "Reopening apps that were quit"
    for bid in "${reopen_ids[@]}"; do
      echo "Opening: $bid"
      open -g -b "$bid" 2>/dev/null || warn "Could not open: $bid"
    done
  fi
else
  warn "brew not found, skipping."
fi

# ---------------------------------------------------------------- Docker: update images of running compose projects
if command -v docker >/dev/null 2>&1 && command -v colima >/dev/null 2>&1 && colima status >/dev/null 2>&1; then
  log "Docker: pull newer images for running compose projects"
  projects=(${(f)"$(list_compose_projects)"})
  if (( ${#projects} > 0 )); then
    for entry in "${projects[@]}"; do
      echo "docker compose pull + up -d: ${entry%%$'\t'*}"
      compose_cmd "$entry" pull --quiet
      compose_cmd "$entry" up -d
    done
  else
    echo "No running compose projects."
  fi

  log "Docker: prune dangling images"
  run docker image prune -f
else
  warn "colima is not running, skipping Docker image updates."
fi

# ---------------------------------------------------------------- uv (installed via brew; uv itself was upgraded above)
if command -v uv >/dev/null 2>&1; then
  log "uv: tool upgrade --all"
  run uv tool upgrade --all

  log "uv: cache prune"
  run uv cache prune
else
  warn "uv not found, skipping."
fi

# ---------------------------------------------------------------- mise
if command -v mise >/dev/null 2>&1; then
  # If installed via brew, mise itself was already upgraded by brew upgrade
  if ! brew list --formula mise >/dev/null 2>&1; then
    log "mise: self-update"
    mise self-update -y || warn "mise self-update skipped/failed"
  fi

  # node@lts is used, so a plain upgrade is enough; do NOT use --bump (it would pin lts to a fixed version)
  log "mise: upgrade (node and other tools)"
  run mise upgrade

  log "mise: prune (remove unused old versions)"
  run mise prune -y

  # Processes still running on a node version that is no longer the active one
  # (e.g. MCP servers spawned by LM Studio / Claude). mise can't prune those until they restart.
  active_node="$(mise where node 2>/dev/null)"
  if [[ -n "$active_node" ]]; then
    mise_node_dir="${active_node:h}"
    stale="$(ps -axo pid=,ppid=,command= | grep -F "$mise_node_dir/" | grep -F -v "$active_node/" | grep -F -v grep)"
    if [[ -n "$stale" ]]; then
      warn "Processes still running on an old node version (restart the app that started them so mise can prune it):"
      print -r -- "$stale" | while read -r pid ppid cmd; do
        parent="$(ps -o comm= -p "$ppid" 2>/dev/null)"
        printf '   - pid %s (started by: %s): %s\n' "$pid" "${parent:t}" "${cmd[1,80]}"
      done
    fi
  fi
else
  warn "mise not found, skipping."
fi

# ---------------------------------------------------------------- summary
log "Done"
if (( ${#failed_steps} > 0 )); then
  warn "Steps that failed:"
  printf '   - %s\n' "${failed_steps[@]}"
  exit 1
fi