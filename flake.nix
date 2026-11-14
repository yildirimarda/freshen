# Nix flake for freshen: one command to keep a developer machine current.
#
# Builds freshen from source with cargo for x86_64-linux and aarch64-linux and
# exposes it as a package, a development shell and a set of CI checks:
#
#   nix run .#freshen              # build and run freshen from this checkout
#   nix develop                    # enter a dev shell with rustc, cargo, pkgconfig
#   nix flake check                # run fmt, clippy and the test suite
#
# The sha256 below is a placeholder. Replace it with the real cargoHash before
# using the flake: build once and copy the hash from the mismatch error, or
# prefetch the crates.io index. See https://nixos.org/manual/nix/stable/language/language.html#builtins-derivation
#
#   cargoHash = "sha256-...";   # from `nix build .#freshen` error message
#
# Publish this flake to a flake registry or a GitHub fork of NixOS/nixpkgs so it
# can be installed with `nix shell github:yildirimarda/freshen#freshen` or
# `nix run github:yildirimarda/freshen#freshen`. (See HUMAN-TASKS.md for the
# accounts and credentials that only a human can create.)

{
  description = "freshen: keep a developer machine up to date on macOS, Linux and Windows";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in rec {
      # Overlay that adds freshen to nixpkgs so other flakes can depend on it.
      overlay = final: prev: {
        freshen = final.rustPlatform.buildRustPackage {
          pname = "freshen";
          version = "0.1.0";
          src = self;

          # sha256 of the vendored Cargo dependencies. Placeholder: the first
          # build fails with the correct hash in its error message.
          cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

          nativeBuildInputs = [ final.pkgconfig ];
        };
      };

      packages = forAllSystems (system:
        (import nixpkgs {
          inherit system;
          overlays = [ self.overlay ];
        }).freshen
      );

      devShells = forAllSystems (system:
        let pkgs = import nixpkgs {
          inherit system;
          overlays = [ self.overlay ];
        };
        in pkgs.mkShell {
          name = "freshen-dev-${system}";
          buildInputs = with pkgs; [ rustc cargo pkgconfig ];
          shellHook = ''
            echo "freshen development shell (${system})"
            echo "Build: cargo build --release   (binary: target/release/freshen)"
            echo "Test:  cargo test --all-features"
          '';
        }
      );

      checks = forAllSystems (system:
        let pkgs = import nixpkgs {
          inherit system;
          overlays = [ self.overlay ];
        };
        in rec {
          fmt = pkgs.runCommand "fmt-${system}" { } ''
            ${pkgs.cargo}/bin/cargo fmt --all -- --check
          '';

          clippy = pkgs.runCommand "clippy-${system}" { } ''
            ${pkgs.cargo}/bin/cargo clippy --all-targets --all-features -- -D warnings
          '';

          test = pkgs.runCommand "test-${system}" { } ''
            ${pkgs.cargo}/bin/cargo test --all-features
          '';
        }
      );
    };
}
