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

      cleanSrc =
        pkgs:
        pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            !(builtins.elem (builtins.baseNameOf path) [
              "target"
              ".git"
              ".jj"
            ])
            && pkgs.lib.cleanSourceFilter path type;
        };

      sourceAuditFor =
        pkgs:
        pkgs.runCommand "nmux-source-audit"
          {
            src = cleanSrc pkgs;
            maxSourceKiB = 64 * 1024;
          }
          ''
          if [ -e "$src/target" ]; then
            echo "flake source unexpectedly contains target/" >&2
            exit 1
          fi

          if [ -e "$src/.git" ] || [ -e "$src/.jj" ]; then
            echo "flake source unexpectedly contains VCS metadata" >&2
            exit 1
          fi

          source_kib="$(du -sk "$src" | cut -f1)"
          if [ "$source_kib" -gt "$maxSourceKiB" ]; then
            echo "flake source is unexpectedly large: ''${source_kib} KiB > ''${maxSourceKiB} KiB" >&2
            exit 1
          fi

          mkdir -p "$out"
          {
            printf 'flake_source_excludes_build_output=passed\n'
            printf 'flake_source_size_kib=%s\n' "$source_kib"
            printf 'flake_source_max_kib=%s\n' "$maxSourceKiB"
          } > "$out/source-audit.txt"
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
