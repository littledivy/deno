{
  description = "Turbocall trampoline benchmark";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        asmFile = if pkgs.stdenv.isAarch64
          then "trampoline_aarch64.s"
          else "trampoline_x86_64.s";

        bench = pkgs.stdenv.mkDerivation {
          name = "turbocall-bench";
          src = ./.;
          nativeBuildInputs = with pkgs; [ clang ];
          buildPhase = ''
            make bench_runner ASM=${asmFile} CC=$CC
          '';
          installPhase = ''
            mkdir -p $out/bin
            cp bench_runner $out/bin/
          '';
        };
      in
      {
        packages.default = bench;

        apps.default = {
          type = "app";
          program = "${bench}/bin/bench_runner";
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [ clang llvm gnumake ];
        };
      });
}
