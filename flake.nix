{
  description = "OBS output that encodes for Discord Go Live, and the Vesktop side that sends it";

  # The same nixpkgs as nixcfg, so the plugin links the ffmpeg and libobs that OBS loads.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/e7439b6b14ad3cc35d05608ebca9bce01a25f5f8";

  outputs =
    { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
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
