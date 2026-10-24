{
  description = "OBS output that encodes for Discord Go Live, and the Vesktop side that sends it";

  # The same nixpkgs as nixcfg, so the plugin links the ffmpeg and libobs that OBS loads.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/e7439b6b14ad3cc35d05608ebca9bce01a25f5f8";

  outputs =
    { self, nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      # `cpuArch` is the -C target-cpu value; nixcfg passes config.mine.cpu.arch.
      # Without it the plugin builds for the generic x86-64 nixpkgs targets.
      lib.plugin =
        {
          pkgs,
          cpuArch ? null,
        }:
        pkgs.rustPlatform.buildRustPackage {
          pname = "obs-discord-live";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./plugin
              ./proto
              ./client
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [
            pkgs.pkg-config
            pkgs.rustPlatform.bindgenHook
          ];
          buildInputs = [
            pkgs.obs-studio
            pkgs.ffmpeg
          ];
          env.RUSTFLAGS = pkgs.lib.optionalString (cpuArch != null) "-C target-cpu=${cpuArch}";
          doCheck = true;
          # OBS's wrapper looks in lib/obs-plugins of every plugin it is given.
          postInstall = ''
            mkdir -p $out/lib/obs-plugins
            mv $out/lib/libobs_discord_live.so $out/lib/obs-plugins/obs-discord-live.so
          '';
          meta = {
            description = "OBS output that encodes for Discord Go Live";
            license = pkgs.lib.licenses.gpl2Plus;
            platforms = pkgs.lib.platforms.linux;
          };
        };

      # The plugin as a Windows DLL for OBS 33.0.0 x64, cross-built with mingw.
      # It links obs.dll and obs-frontend-api.dll from that release, and the
      # FFmpeg import libraries of the obs-deps build that release ships, so
      # it loads only DLLs OBS already has.
      lib.pluginWindows =
        {
          pkgs,
          cpuArch ? null,
        }:
        let
          cross = pkgs.pkgsCross.mingwW64;
          obsSrc = pkgs.fetchFromGitHub {
            owner = "obsproject";
            repo = "obs-studio";
            rev = "33.0.0";
            hash = "sha256-ecDFtBRZAlRILPZLRgZLjMxf5VdYyPqG9VEkEnkLowA=";
          };
          obsRelease = pkgs.fetchzip {
            url = "https://github.com/obsproject/obs-studio/releases/download/33.0.0/OBS-Studio-33.0.0-Windows-x64.zip";
            hash = "sha256-1YPLetn2XiS17HkJFMbn9Gc8cjjHdgpLCKPoaRmLYC4=";
            stripRoot = false;
          };
          # The pin in OBS 33.0.0's CMakePresets.json.
          obsDeps = pkgs.fetchzip {
            url = "https://github.com/obsproject/obs-deps/releases/download/2026-10-23/windows-deps-2026-10-23-x64.zip";
            hash = "sha256-/z0ceeTRiUOrl8TMzovWEDssRqh8JMl5BFObq6vq5xQ=";
            stripRoot = false;
          };
          # clang 21's AMX headers give bindgen a struct with both packed and
          # aligned for the Windows target, which rustc rejects; nothing here uses AMX.
          noAmx = map (g: "-D${g}") [
            "__AMXINTRIN_H"
            "__AMX_AVX512INTRIN_H"
            "__AMX_BF16TRANSPOSEINTRIN_H"
            "__AMX_COMPLEXINTRIN_H"
            "__AMX_COMPLEXTRANSPOSEINTRIN_H"
            "__AMX_FP16INTRIN_H"
            "__AMX_FP16TRANSPOSEINTRIN_H"
            "__AMXFP8INTRIN_H"
            "__AMXMOVRSINTRIN_H"
            "__AMX_MOVRS_TRANSPOSEINTRIN_H"
            "__AMX_TF32INTRIN_H"
            "__AMX_TF32TRANSPOSEINTRIN_H"
            "__AMX_TRANSPOSEINTRIN_H"
          ];
        in
        cross.rustPlatform.buildRustPackage {
          pname = "obs-discord-live-windows";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./plugin
              ./proto
              ./client
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "obs-discord-live" ];
          nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
          buildInputs = [ cross.windows.pthreads ];
          env = {
            OBS_SRC = obsSrc;
            OBS_BIN = "${obsRelease}/bin/64bit";
            FFMPEG_DIR = obsDeps;
            BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnu = toString (
              [
                "--target=x86_64-pc-windows-gnu"
                "-isystem ${cross.windows.mingw_w64_headers}/include"
              ]
              ++ noAmx
            );
            RUSTFLAGS = pkgs.lib.optionalString (cpuArch != null) "-C target-cpu=${cpuArch}";
          };
          doCheck = false;
          installPhase = ''
            runHook preInstall
            d=$out/obs-plugins/obs-discord-live/bin/64bit
            mkdir -p $d
            cp target/x86_64-pc-windows-gnu/release/obs_discord_live.dll $d/obs-discord-live.dll
            runHook postInstall
          '';
          meta.license = pkgs.lib.licenses.gpl2Plus;
        };

      # Vesktop with OBS as a Go Live source. The lockfile is unchanged, so
      # nixpkgs's pnpm dependency hash still holds.
      lib.vesktop =
        { pkgs }:
        pkgs.vesktop.overrideAttrs (old: {
          patches = (old.patches or [ ]) ++ [ ./vesktop/obs-discord-live.patch ];
        });

      packages.${system} = {
        default = self.lib.plugin { inherit pkgs; };
        vesktop = self.lib.vesktop { inherit pkgs; };
        windows = self.lib.pluginWindows { inherit pkgs; };
      };

      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = [
          pkgs.pkg-config
          pkgs.rustPlatform.bindgenHook
        ];
        buildInputs = [
          pkgs.obs-studio
          pkgs.ffmpeg
        ];
        packages = [ pkgs.ffmpeg.bin ];
      };
    };
}
