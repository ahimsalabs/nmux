{
  description = "nmux development environment";

  inputs.crane.url = "github:ipetkov/crane";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    {
      self,
      crane,
      nixpkgs,
      ...
    }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];

      forEachSystem = nixpkgs.lib.genAttrs systems;

      packageVersion = "0.1.0";

      nixFormatterFor =
        pkgs:
        pkgs.writeShellApplication {
          name = "nmux-nixfmt";
          runtimeInputs = [
            pkgs.nixfmt
          ];
          text = ''
            if [ "$#" -eq 0 ]; then
              exec nixfmt flake.nix
            fi
            exec nixfmt "$@"
          '';
        };

      cleanSrc =
        pkgs:
        pkgs.lib.cleanSourceWith {
          src = ./.;
          filter =
            path: type:
            let
              name = builtins.baseNameOf path;
            in
            !(builtins.elem name [
              "target"
              ".git"
              ".jj"
              "result"
            ])
            && !(pkgs.lib.hasPrefix "result-" name)
            && pkgs.lib.cleanSourceFilter path type;
        };

      defaultNativeBuildInputs = pkgs: [
        pkgs.flatbuffers
      ];

      defaultBuildArgs = pkgs: {
        pname = "nmux";
        version = packageVersion;
        src = cleanSrc pkgs;
        strictDeps = true;
        cargoExtraArgs = "-p nmux-cli --bin nmux";
        nativeBuildInputs = defaultNativeBuildInputs pkgs;
      };

      defaultPackageFor =
        pkgs:
        let
          craneLib = crane.mkLib pkgs;
          commonArgs = defaultBuildArgs pkgs;
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;
        in
        craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;
            doCheck = false;
            meta.mainProgram = "nmux";
          }
        );

      defaultTestsFor =
        pkgs:
        let
          craneLib = crane.mkLib pkgs;
          commonArgs = defaultBuildArgs pkgs;
          cargoArtifacts = craneLib.buildDepsOnly (
            commonArgs
            // {
              cargoExtraArgs = "--workspace";
            }
          );
        in
        craneLib.cargoTest (
          commonArgs
          // {
            inherit cargoArtifacts;
            cargoExtraArgs = "--workspace --no-run";
            RUST_TEST_THREADS = "1";
            preCheck = ''
              export PATH=${pkgs.bash}/bin:$PATH
              flatc --json --strict-json --no-warnings -o /tmp schema/nmux.fbs
            '';
          }
        );

      sourceAuditFor =
        pkgs:
        pkgs.runCommand "nmux-source-audit"
          {
            src = cleanSrc pkgs;
            flakeSrc = self.outPath;
            maxSourceKiB = 64 * 1024;
            maxFlakeSourceKiB = 64 * 1024;
          }
          ''
            if [ -e "$flakeSrc/target" ]; then
              echo "flake input source unexpectedly contains target/" >&2
              exit 1
            fi

            if [ -e "$flakeSrc/result" ] || find "$flakeSrc" -maxdepth 1 -name 'result-*' -print -quit | grep -q .; then
              echo "flake input source unexpectedly contains Nix result symlinks" >&2
              exit 1
            fi

            if [ -e "$flakeSrc/.git" ] || [ -e "$flakeSrc/.jj" ]; then
              echo "flake input source unexpectedly contains VCS metadata" >&2
              exit 1
            fi

            if [ -e "$src/target" ]; then
              echo "flake source unexpectedly contains target/" >&2
              exit 1
            fi

            if [ -e "$src/result" ] || find "$src" -maxdepth 1 -name 'result-*' -print -quit | grep -q .; then
              echo "flake source unexpectedly contains Nix result symlinks" >&2
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

            flake_source_kib="$(du -sk "$flakeSrc" | cut -f1)"
            if [ "$flake_source_kib" -gt "$maxFlakeSourceKiB" ]; then
              echo "flake input source is unexpectedly large: ''${flake_source_kib} KiB > ''${maxFlakeSourceKiB} KiB" >&2
              exit 1
            fi

            mkdir -p "$out"
            {
              printf 'flake_input_source_excludes_build_output=passed\n'
              printf 'flake_input_source_excludes_result_links=passed\n'
              printf 'flake_input_source_size_kib=%s\n' "$flake_source_kib"
              printf 'flake_input_source_max_kib=%s\n' "$maxFlakeSourceKiB"
              printf 'flake_source_excludes_build_output=passed\n'
              printf 'flake_source_excludes_result_links=passed\n'
              printf 'flake_source_size_kib=%s\n' "$source_kib"
              printf 'flake_source_max_kib=%s\n' "$maxSourceKiB"
            } > "$out/source-audit.txt"
          '';

      packageSmokeFor =
        pkgs: package:
        pkgs.runCommand "nmux-package-smoke" { } ''
          test -x ${package}/bin/nmux
          ${package}/bin/nmux --version | grep -F 'nmux ${packageVersion}'

          mkdir -p "$out"
          {
            printf 'nmux_package_binary=passed\n'
            printf 'nmux_package_version=passed\n'
          } > "$out/package-smoke.txt"
        '';
    in
    {
      packages = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = defaultPackageFor pkgs;
        }
      );

      formatter = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        nixFormatterFor pkgs
      );

      devShells = forEachSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [
              self.packages.${system}.default
            ];
            packages = [
              pkgs.cargo
              pkgs.cargo-zigbuild
              pkgs.flatbuffers
              pkgs.gnumake
              pkgs.nixfmt
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
        rec {
          default = nmux-tests;
          nmux-tests = defaultTestsFor pkgs;
          nmux-package = self.packages.${system}.default;
          nmux-package-smoke = packageSmokeFor pkgs self.packages.${system}.default;
          source-audit = sourceAuditFor pkgs;
        }
      );
    };
}
