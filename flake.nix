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
