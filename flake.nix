{
  description = "A tool for deploying a nix flake to remote systems";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nmattia/naersk";
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , naersk
    , ...
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = nixpkgs.legacyPackages.${system};
      naersk-lib = naersk.lib."${system}";
      nativeBuildInputs = [ pkgs.libiconv ];
    in
    rec {
      packages = {
        deploy-flake = naersk-lib.buildPackage {
          inherit nativeBuildInputs;
          root = ./.;
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
