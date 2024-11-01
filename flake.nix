{
  description = "A tool for deploying a nix flake to remote systems";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix?ref=main";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    gitignore = {
      url = "github:hercules-ci/gitignore.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    fenix,
    gitignore,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import nixpkgs {
        inherit system;
      };
      nativeBuildInputs = [pkgs.libiconv];
    in {
      packages = {
        deploy-flake = let
          rustPlatform = pkgs.makeRustPlatform {
            inherit (fenix.packages.${system}.stable) rustc cargo;
          };
          inherit (gitignore.lib) gitignoreSource;
        in
          rustPlatform.buildRustPackage {
            pname = "deploy-flake";
            version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            inherit nativeBuildInputs;
            src = gitignoreSource ./.;
            cargoLock.lockFile = ./Cargo.lock;
          };

        default = self.packages.${system}.deploy-flake;
      };

      formatter = pkgs.alejandra;

      apps = {
        deploy-flake = {
          type = "app";
          program = "${self.packages."${system}".deploy-flake}/bin/deploy-flake";
        };
        default = self.apps.${system}.deploy-flake;
      };
    });
}
