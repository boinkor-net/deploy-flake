{
  description = "A tool for deploying a nix flake to remote systems";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};
      nativeBuildInputs = [ pkgs.libiconv ];
    in
    rec {
      packages = {
        deploy-flake = pkgs.rustPlatform.buildRustPackage rec {
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
