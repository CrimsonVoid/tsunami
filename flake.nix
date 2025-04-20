{
  description = "tsunami bittorrent client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, fenix }:
    let
      archs = [ "x86_64-linux" "x86_64-darwin" "aarch64-linux" "aarch64-darwin" ];
      sysPkgs = nixpkgs.lib.genAttrs archs (s: import nixpkgs { system = s; });
      genSystems = fn: nixpkgs.lib.genAttrs archs (s: fn s sysPkgs.${s});
    in
    {
      devShells = genSystems (system: pkgs: {
        default = pkgs.mkShell {
          name = "tsunami";

          buildInputs = with pkgs; [
            fenix.packages.${system}.latest.toolchain
            pkg-config
            openssl
          ];
        };
      });

      formatter = genSystems (_: pkgs: pkgs.nixpkgs-fmt);
    };
}

