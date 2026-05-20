{
  description = "pgvis — Storyvis AI's Rust PostgREST port";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        # Read the toolchain from rust-toolchain.toml so Nix and cargo agree
        rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

        # Crane for building the Rust workspace
        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Common arguments for crane builds
        commonArgs = {
          src = craneLib.cleanCargoSource ./.;
          strictDeps = true;

          buildInputs = [
            pkgs.openssl
            pkgs.postgresql
          ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
            pkgs.libiconv
          ];

          nativeBuildInputs = [
            pkgs.pkg-config
          ];
        };

        # Build just the cargo dependencies for caching
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the full workspace
        pgvis = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      {
        # `nix build`
        packages = {
          default = pgvis;
          pgvis = pgvis;
        };

        # `nix run`
        apps.default = flake-utils.lib.mkApp {
          drv = pgvis;
        };

        # `nix flake check`
        checks = {
          inherit pgvis;

          pgvis-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          pgvis-fmt = craneLib.cargoFmt {
            src = craneLib.cleanCargoSource ./.;
          };

          pgvis-nextest = craneLib.cargoNextest (commonArgs // {
            inherit cargoArtifacts;
            partitions = 1;
            partitionType = "count";
          });
        };

        # `nix develop`
        devShells.default = craneLib.devShell {
          # Extra inputs on top of what crane already provides (rustc, cargo, etc.)
          packages = with pkgs; [
            # Rust extras
            rust-analyzer
            cargo-watch
            cargo-nextest
            cargo-insta       # snapshot testing
            cargo-expand
            cargo-deny
            cargo-audit
            cargo-machete     # detect unused deps

            # Database tooling
            postgresql
            pgcli
            postgrest

            # Node.js (for website)
            nodejs
            wrangler

            # Nix / general
            nil               # Nix LSP
            nixpkgs-fmt
            direnv
            just              # task runner

            # Observability / debug
            jq
            curl
            httpie
          ];

          # Environment variables available in the dev shell
          shellHook = ''
            echo "🔧 pgvis dev shell — $(rustc --version)"
            export PGVIS_DSN="''${PGVIS_DSN:-postgres://localhost:5432/pgvis}"
            export RUST_LOG="''${RUST_LOG:-pgvis=debug,tower_http=debug}"
            export RUST_BACKTRACE=1
          '';
        };
      }
    );
}
