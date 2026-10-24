#![allow(unused)]
//! Shared transcript helpers for container runtime stub tests.

pub const ALL_STEPS: &str =
    "brew,brew-formulae,brew-services,brew-casks,brew-autoremove,brew-cleanup,brew-reopen,colima-stop,colima-start,docker";

pub const PS_CALL: &str =
    "docker ps --format {{.ID}}\t{{.Names}}\t{{.Label \"com.docker.compose.project\"}}";

pub const COMPOSE_JSON: &str = r#"[
    {"Name":"web","ConfigFiles":"/srv/web/compose.yml,/srv/web/override.yml"},
    {"Name":"db","ConfigFiles":"/srv/db/compose.yml"}
]"#;

pub const PS_OUTPUT: &str = "abc\tweb-1\tweb\ndef\tlonely\t\nghi\tlonely2\t\n";

pub fn compose(project: &str, files: &[&str], action: &str) -> String {
    let mut call = format!("docker compose -p {project}");
    for file in files {
        call.push_str(" -f ");
        call.push_str(file);
    }
    call.push(' ');
    call.push_str(action);
    call
}

pub fn web(action: &str) -> String {
    compose(
        "web",
        &["/srv/web/compose.yml", "/srv/web/override.yml"],
        action,
    )
}

pub fn db(action: &str) -> String {
    compose("db", &["/srv/db/compose.yml"], action)
}

// Named insertions: transcript pieces used across container tests.

pub fn docker_version() -> &'static [&'static str] {
    &["docker compose version"]
}

pub fn brew_update() -> &'static [&'static str] {
    &["brew update"]
}

pub fn brew_formula_query() -> &'static [&'static str] {
    &["brew outdated --formula --quiet"]
}

pub fn brew_formula_upgrade() -> &'static [&'static str] {
    &[
        "brew outdated --formula",
        "brew outdated --formula --quiet",
        "brew list --pinned --formula",
        "brew services list",
        "brew upgrade --formula",
    ]
}

pub fn brew_cask_query() -> &'static [&'static str] {
    &[
        "brew outdated --cask --greedy",
        "brew outdated --cask --greedy --quiet",
    ]
}

pub fn brew_autoremove() -> &'static [&'static str] {
    &["brew autoremove --dry-run", "brew autoremove"]
}

pub fn brew_cleanup() -> &'static [&'static str] {
    &[
        "brew cleanup --dry-run --prune=all",
        "brew cleanup --prune=all",
    ]
}

pub fn colima_stop() -> &'static [&'static str] {
    &["colima stop"]
}

pub fn colima_start() -> &'static [&'static str] {
    &["colima start"]
}

pub fn docker_stop_containers() -> &'static [&'static str] {
    &["docker stop def", "docker stop ghi"]
}

pub fn docker_start_containers() -> &'static [&'static str] {
    &["docker start def", "docker start ghi"]
}

pub fn docker_image_prune() -> &'static [&'static str] {
    &["docker image prune -f"]
}

pub fn limactl_stop(name: &str) -> String {
    format!("limactl stop {name}")
}

pub fn limactl_start(name: &str) -> String {
    format!("limactl start {name}")
}

pub fn limactl_list_json() -> &'static [&'static str] {
    &["limactl list --json"]
}
