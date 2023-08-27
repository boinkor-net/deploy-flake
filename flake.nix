{
  description = "A tool for deploying a nix flake to remote systems";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , rust-overlay
    , gitignore
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
            inherit (gitignore.lib) gitignoreSource;
          in
          rustPlatform.buildRustPackage rec {
            pname = "deploy-flake";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            inherit nativeBuildInputs;
            src = gitignoreSource ./.;
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
