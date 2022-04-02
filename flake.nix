{
  description = "A tool for deploying a nix flake to remote systems";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , rust-overlay
    , ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = (import nixpkgs { inherit system; overlays = [ (import rust-overlay) ]; });
      nativeBuildInputs = [ pkgs.libiconv ];
    in
    rec {
      packages = {
        deploy-flake =
          let
            rustPlatform = pkgs.makeRustPlatform {
              rustc = pkgs.rust-bin.stable.latest.minimal;
              cargo = pkgs.rust-bin.stable.latest.minimal;
            };
          in
          rustPlatform.buildRustPackage rec {
            pname = "deploy-flake";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            inherit nativeBuildInputs;
            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };
      };

      apps = {
        deploy-flake = {
          type = "app";
          program = "${self.packages."${system}".deploy-flake}/bin/deploy-flake";
        };
      };
    });
}
