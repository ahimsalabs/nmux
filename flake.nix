{
  description = "nmux development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs, ... }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      forEachSystem = nixpkgs.lib.genAttrs systems;

      sourceAuditFor =
        pkgs:
        pkgs.runCommand "nmux-source-audit" { src = self; } ''
          if [ -e "$src/target" ]; then
            echo "flake source unexpectedly contains target/" >&2
            exit 1
          fi

          if [ -e "$src/.git" ] || [ -e "$src/.jj" ]; then
            echo "flake source unexpectedly contains VCS metadata" >&2
            exit 1
          fi

          mkdir -p "$out"
          printf 'flake_source_excludes_build_output=passed\n' > "$out/source-audit.txt"
        '';
    in
    {
      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.flatbuffers
              pkgs.gnumake
              pkgs.rustc
              pkgs.rustfmt
              pkgs.zig_0_15
            ];
          };
        }
      );

      checks = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          source-audit = sourceAuditFor pkgs;
        }
      );
    };
}
