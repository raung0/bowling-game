{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      fenix,
      ...
    }:
    let
      overlay =
        final: prev:
        let
          fenixPkgs = fenix.packages.${final.stdenv.hostPlatform.system};
        in
        {
          rustToolchain =
            with fenixPkgs;
            combine (
              with latest;
              [
                clippy
                rustc
                cargo
                rustfmt
                rust-src

                targets.wasm32-unknown-emscripten.latest.rust-std
              ]
            );
        };
    in
    {
      overlays.default = overlay;
    }
    // flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ overlay ];
        };
      in
      {
        packages =
          let
            server = pkgs.rustPlatform.buildRustPackage {
              pname = "server";
              version = "0.1.0";

              src = ./.;
              cargoRoot = "rust";
              buildAndTestSubdir = "rust/crates/server";
              cargoBuildFlags = [ "-p" "server" ];

              cargoLock = {
                lockFile = ./rust/Cargo.lock;
                allowBuiltinFetchGit = true;
              };

              nativeBuildInputs = with pkgs; [
                rustToolchain
                pkg-config
                emscripten
                godot_4
                godot_4-export-templates-bin
                python3
              ];

              buildInputs = with pkgs; [
                openssl
              ];

              preBuild = ''
                export HOME="$TMPDIR/home"
                export XDG_DATA_HOME="$HOME/.local/share"
                export XDG_CONFIG_HOME="$HOME/.config"
                templates_home_xdg="$XDG_DATA_HOME/godot/export_templates"
                templates_home_macos="$HOME/Library/Application Support/Godot/export_templates"
                templates_src="${pkgs.godot_4-export-templates-bin}/share/godot/export_templates"
                mkdir -p "$templates_home_xdg" "$templates_home_macos"
                for version_dir in "$templates_src"/*; do
                  if [ -d "$version_dir" ]; then
                    ln -s "$version_dir" "$templates_home_xdg/$(basename "$version_dir")"
                    ln -s "$version_dir" "$templates_home_macos/$(basename "$version_dir")"
                  fi
                done
                bash "$NIX_BUILD_TOP/$sourceRoot/scripts/make_web.sh"
              '';

              env = {
                CC_wasm32_unknown_emscripten = "emcc";
                CXX_wasm32_unknown_emscripten = "em++";
              };
            };
          in
          {
            inherit server;
            default = server;
          };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            rustToolchain
            openssl
            pkg-config
            cargo-deny
            cargo-edit
            cargo-watch
            rust-analyzer
            emscripten
            python3
            godot_4
            caddy
          ];

          env = {
            RUST_SRC_PATH = "${pkgs.rustToolchain}/lib/rustlib/src/rust/library";
            CC_wasm32_unknown_emscripten = "emcc";
            CXX_wasm32_unknown_emscripten = "em++";
          };
        };
      }
    );
}
