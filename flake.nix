{
  description = "Bubblewrap namespace sandbox as a home-manager module";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
    home-manager = {
      url = "github:nix-community/home-manager/release-25.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      home-manager,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;

    in
    {
      homeManagerModules.cloister = import ./modules/cloister;
      homeManagerModules.default = self.homeManagerModules.cloister;

      nixosModules.cloister-netns = import ./modules/cloister-netns;

      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          cloister-netns = pkgs.callPackage ./helpers/cloister-netns { };
          cloister-wayland-validate = pkgs.callPackage ./helpers/cloister-wayland-validate { };
          cloister-dbus-validate = pkgs.callPackage ./helpers/cloister-dbus-validate { };
          cloister-seccomp-filter = pkgs.callPackage ./helpers/cloister-seccomp-filter { };
          cloister-seccomp-validate = pkgs.callPackage ./helpers/cloister-seccomp-validate { };
          cloister-sandbox = pkgs.callPackage ./helpers/cloister-sandbox { };
        }
      );

      checks = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        import ./tests {
          inherit pkgs;
          inherit home-manager;
          cloister-module = import ./modules/cloister;
        }
      );
    };
}
