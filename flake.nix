{
  description = "Ncs development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        # Rust toolchain — stable with common components
        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
            "rustfmt"
            "clippy"
          ];
          targets = [ ];
        };

        # Common build inputs for ratatui / reqwest / TUI apps on NixOS
        buildInputs = with pkgs; [
          # TLS for reqwest
          openssl
          pkg-config

          # TUI deps
          ncurses

          # Tools used by the agent at runtime
          nushell
          ripgrep
          jujutsu
        ];

        # Development-only tools (not needed at runtime)
        devTools = with pkgs; [
          # rust
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer
          cargo-watch
          cargo-nextest
          cargo-expand
          bacon # background rust compiler
          git
        ];

      in
      {
        devShells.default = pkgs.mkShell {
          name = "n-coding-dev";

          nativeBuildInputs = buildInputs ++ devTools;

          shellHook = ''
            echo " N-coding dev shell"
            echo "   rustc  : $(rustc --version)"
            echo "   cargo  : $(cargo --version)"
            echo "   nu     : $(nu --version 2>/dev/null || echo 'not found')"
            echo "   rg     : $(rg --version 2>/dev/null || echo 'not found')"
            echo "   jj     : $(jj --version 2>/dev/null || echo 'not found')"
            echo ""

            export RUST_BACKTRACE=1
            export OPENSSL_DIR="${pkgs.openssl.dev}"
            export OPENSSL_LIB_DIR="${pkgs.openssl.out}/lib"

            # 启动Nushell
            exec nu --no-config-file --no-history
          '';

          # Environment variables for building/running
          RUSTFLAGS = "-C target-cpu=native";
          CARGO_TERM_COLOR = "always";
        };

        # Minimal shell for CI or non-interactive builds
        devShells.ci = pkgs.mkShell {
          name = "n-coding-ci";
          nativeBuildInputs = buildInputs;
          RUSTFLAGS = "-C target-cpu=native";
          CARGO_TERM_COLOR = "always";
        };
      }
    );
}
