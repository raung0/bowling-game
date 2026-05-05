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
            godot_4
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
