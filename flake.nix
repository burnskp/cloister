{
  description = "Bubblewrap namespace sandbox as a home-manager module";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    # Real Home Manager evaluation for the published standalone example.
    home-manager = {
      url = "github:nix-community/home-manager";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      treefmt-nix,
      home-manager,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;

      treefmtEval = forAllSystems (
        system:
        treefmt-nix.lib.evalModule nixpkgs.legacyPackages.${system} {
          projectRootFile = "flake.nix";
          programs = {
            deadnix.enable = true;
            mdformat.enable = true;
            nixfmt = {
              enable = true;
              package = nixpkgs.legacyPackages.${system}.nixfmt;
            };
            rustfmt.enable = true;
            statix.enable = true;
            toml-sort.enable = true;
          };
        }
      );

    in
    {
      homeManagerModules.cloister = import ./modules/cloister;
      homeManagerModules.default = self.homeManagerModules.cloister;

      nixosModules.cloister-netns = import ./modules/cloister-netns;
      nixosModules.cloister-image-store = import ./modules/cloister-image-store;

      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          bubblewrap-subset-pid = pkgs.callPackage ./pkgs/bubblewrap-subset-pid { };
          inherit (pkgs) cargo-audit;
          cloister-netns = pkgs.callPackage ./helpers/cloister-netns { };
          cloister-diagnostics = pkgs.callPackage ./pkgs/cloister-diagnostics { };
          cloister-seccomp-filter = pkgs.callPackage ./helpers/cloister-seccomp-filter { };
          cloister-sandbox = pkgs.callPackage ./helpers/cloister-sandbox { };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          cargoAudit = self.packages.${system}.cargo-audit;
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              cargo
              cargoAudit
              rustc
              clippy
              clang
              pkg-config
            ];
            buildInputs = with pkgs; [
              libclang
              libseccomp
              pipewire
              wayland
            ];

            LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
          };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          testChecks = import ./tests {
            inherit pkgs;
            inherit (nixpkgs) lib;
          };
        in
        {
          treefmt = treefmtEval.${system}.config.build.check self;
          test-validation =
            pkgs.runCommand "cloister-validation"
              {
                nativeBuildInputs = [
                  pkgs.python3
                  pkgs.gnumake
                  pkgs.git
                ];
              }
              ''
                python ${self}/tests/validation.py
                touch $out
              '';
          test-cloister-standalone-example = import ./tests/cloister/standalone-example.nix {
            inherit pkgs home-manager nixpkgs;
            cloister = self;
          };
        }
        // nixpkgs.lib.mapAttrs' (
          name: value: nixpkgs.lib.nameValuePair "package-${name}" value
        ) self.packages.${system}
        // testChecks
      );

      formatter = forAllSystems (system: treefmtEval.${system}.config.build.wrapper);
    };
}
