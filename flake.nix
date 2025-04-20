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
      genSystems = nixpkgs.lib.genAttrs archs;
      sysPkgs = genSystems (system: import nixpkgs { inherit system; });
    in
    {
      devShells = genSystems (system:
        let
          pkgs = sysPkgs.${system};
        in
        {
          default = pkgs.mkShell {
            name = "tsunami";

            buildInputs = with pkgs; [
              fenix.packages.${system}.latest.toolchain
              pkg-config
              openssl
            ];
          };
        });

      formatter = genSystems (system: sysPkgs.${system}.nixpkgs-fmt);
    };
}
