{
  description = "rexia: a native Linux client for Rex, built on GPUI and themed by Noctalia";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.crane.url = "github:ipetkov/crane";

  outputs =
    {
      self,
      nixpkgs,
      crane,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in
    {
      packages = forAllSystems (
        system:
        let
          rexia = (pkgsFor system).callPackage ./nix/package.nix {
            craneLib = crane.mkLib (pkgsFor system);
          };
        in
        {
          inherit rexia;
          default = rexia;
        }
      );

      apps = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          # Development commands run from the checkout inside the dev shell, so they
          # edit and build the working tree rather than a store copy.
          sourceApp =
            name: description: command:
            let
              wrapper = pkgs.writeShellApplication {
                name = "rexia-${name}";
                runtimeInputs = [ pkgs.nix ];
                text = ''
                  if [[ ! -f Cargo.toml || ! -f flake.nix ]]; then
                    echo 'Run this command from the rexia checkout root.' >&2
                    exit 1
                  fi
                  exec nix develop --command ${command} "$@"
                '';
              };
            in
            {
              type = "app";
              program = "${wrapper}/bin/rexia-${name}";
              meta.description = description;
            };
          rexia = {
            type = "app";
            program = "${self.packages.${system}.rexia}/bin/rexia";
            meta.description = "Run the rexia Rex client";
          };
        in
        {
          default = rexia;
          inherit rexia;
          dev = sourceApp "dev" "Run rexia from source" "cargo run --locked --";
          build = sourceApp "build" "Build the release binary" "cargo build --locked --release";
          test = sourceApp "test" "Run the test suite" "cargo test --locked";
          lint =
            sourceApp "lint" "Run clippy with warnings denied"
              "cargo clippy --locked --all-targets -- -D warnings";
          fmt = sourceApp "fmt" "Format Rust sources" "cargo fmt";
          fmt-check = sourceApp "fmt-check" "Check Rust formatting" "cargo fmt --check";
          check =
            sourceApp "check" "Run formatting, lint and tests"
              "sh -c 'cargo fmt --check && cargo clippy --locked --all-targets -- -D warnings && cargo test --locked'";
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
          package = self.packages.${system}.rexia;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ package ];
            packages = with pkgs; [
              rustc
              cargo
              rustfmt
              clippy
              rust-analyzer
              python3
              nushell
              nixfmt
              dbus
            ];
            inherit (package) GHOSTTY_SOURCE_DIR GHOSTTY_ZIG_SYSTEM_DIR;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath package.passthru.runtimeLibraries;
            shellHook = ''
              export ZIG_GLOBAL_CACHE_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/rexia/zig"
              mkdir -p "$ZIG_GLOBAL_CACHE_DIR"
            '';
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = pkgsFor system;
        in
        {
          inherit (self.packages.${system}) rexia;
          nix-format = pkgs.runCommand "rexia-nix-format" { nativeBuildInputs = [ pkgs.nixfmt ]; } ''
            nixfmt --check ${./flake.nix} ${./nix/package.nix}
            touch "$out"
          '';
          rust-format = pkgs.runCommand "rexia-rust-format" { nativeBuildInputs = [ pkgs.rustfmt ]; } ''
            rustfmt --check --edition 2024 ${./src/main.rs}
            touch "$out"
          '';
        }
      );

      formatter = forAllSystems (system: (pkgsFor system).nixfmt-tree);
    };
}
