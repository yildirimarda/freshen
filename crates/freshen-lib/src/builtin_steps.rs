//! Every built-in step, in registration order.
//!
//! A single source of truth for which steps exist: the `freshen` binary uses
//! it to build its run registry, and the docs generator uses it to produce one
//! page per step. Custom steps from the configuration file are added on top by
//! the binary itself.

use crate::registry::Registry;

/// Registers every built-in step on `registry`, in the order they run, and
/// returns the registry for chaining.
///
/// `runtime` is the container runtime detected on the host (or `None` when no
/// specific runtime is needed, for example when generating documentation).
pub fn register_all(
    registry: Registry,
    runtime: Option<crate::containers::ContainerRuntimeKind>,
) -> Registry {
    let registry = crate::containers::register_for(registry, runtime);
    let registry = crate::homebrew::register(registry);
    let registry = crate::antidote::register(registry);
    let registry = crate::aws::register(registry);
    let registry = crate::az::register(registry);
    let registry = crate::aur::register(registry);
    let registry = crate::emerge::register(registry);
    let registry = crate::apk::register(registry);
    let registry = crate::zypper::register(registry);
    let registry = crate::pacman::register(registry);
    let registry = crate::dnf::register(registry);
    let registry = crate::apt::register(registry);
    let registry = crate::termux::register(registry);
    let registry = crate::rpm_ostree::register(registry);
    let registry = crate::bootc::register(registry);
    let registry = crate::flatpak::register(registry);
    let registry = crate::fisher::register(registry);
    let registry = crate::firmware::register(registry);
    let registry = crate::gcloud::register(registry);
    let registry = crate::snap::register(registry);
    let registry = crate::nix::register(registry);
    let registry = crate::tooling::register(registry);
    let registry = crate::elixir::register(registry);
    let registry = crate::composer::register(registry);
    let registry = crate::conda::register(registry);
    let registry = crate::mamba::register(registry);
    let registry = crate::nvm::register(registry);
    let registry = crate::nvim::register(registry);
    let registry = crate::omz::register(registry);
    let registry = crate::zinit::register(registry);
    let registry = crate::tpm::register(registry);
    let registry = crate::packer::register(registry);
    let registry = crate::mason::register(registry);
    let registry = crate::treesitter::register(registry);
    let registry = crate::fnm::register(registry);
    let registry = crate::volta::register(registry);
    let registry = crate::pip::register(registry);
    let registry = crate::pipx::register(registry);
    let registry = crate::rustup::register(registry);
    let registry = crate::sdkman::register(registry);
    let registry = crate::dotnet_tools::register(registry);
    let registry = crate::jenv::register(registry);
    let registry = crate::tfenv::register(registry);
    let registry = crate::asdf::register(registry);
    let registry = crate::cargo_update::register(registry);
    let registry = crate::pnpm::register(registry);
    let registry = crate::yarn::register(registry);
    let registry = crate::bun::register(registry);
    let registry = crate::pyenv::register(registry);
    let registry = crate::rbenv::register(registry);
    let registry = crate::gem::register(registry);
    let registry = crate::go::register(registry);
    let registry = crate::appstore::register(registry);
    let registry = crate::vscode::register(registry);
    let registry = crate::cursor::register(registry);
    let registry = crate::vscodium::register(registry);
    let registry = crate::gh::register(registry);
    let registry = crate::git_lfs::register(registry);
    let registry = crate::pre_commit::register(registry);
    let registry = crate::helm::register(registry);
    let registry = crate::kubectl::register(registry);
    let registry = crate::winget::register(registry);
    let registry = crate::chocolatey::register(registry);
    let registry = crate::microsoft_store::register(registry);
    let registry = crate::scoop::register(registry);
    let registry = crate::wsl::register(registry);
    crate::windows_update::register(registry)
}
