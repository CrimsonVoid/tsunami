{
  description = "a c++ project";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rustovl.url = "github:oxalica/rust-overlay";
  };

  outputs = { self, nixpkgs, rustovl }:
    let
      archs =
        [ "x86_64-linux" "x86_64-darwin" "aarch64-linux" "aarch64-darwin" ];
      genSystems = nixpkgs.lib.genAttrs archs;

      overlays = [ (import rustovl) ];
      sysPkgs = genSystems (system: import nixpkgs { inherit system overlays; });
    in
    {
      devShells = genSystems (system:
        let
          pkgs = sysPkgs.${system};
          llvm = pkgs.llvmPackages_16;
        in
        {
          default = pkgs.mkShell {
            name = "tsunami";

            buildInputs = with pkgs; [ rust-bin.nightly.latest.default ];
          };
        });

      formatter = genSystems (system: sysPkgs.${system}.nixpkgs-fmt);
    };
}
