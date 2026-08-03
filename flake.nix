{
  description = "A chart manager for UNBEATABLE";

  inputs = {
    # currently gstreamer & vitejs seem to be broken on unstable
    # so i guess 26.05 will have to do for now
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

    crane.url = "github:ipetkov/crane";
    crane-tauri.url = "github:JPHutchins/crane-tauri";

  };

  outputs =
    { ... }@inputs:
    let
      inherit (inputs.nixpkgs) lib;

      systems = [ "x86_64-linux" ];
      eachSystem = lib.genAttrs systems;

      pkgsFor = eachSystem (
        system:
        import inputs.nixpkgs {
          inherit system;
        }
      );
    in
    {
      formatter = eachSystem (system: inputs.nixpkgs.legacyPackages.${system}.nixfmt-tree);

      devShells = eachSystem (
        system:
        let
          pkgs = pkgsFor.${system};
        in
        {
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              pkg-config
              wrapGAppsHook3
              cargo
              cargo-tauri
              nodejs
              rustc # Needed for dev server (npm tauri dev)

              vitejs
              typescript
            ];

            buildInputs = with pkgs; [
              openssl
              webkitgtk_4_1

              # audio & video playback (track previews, etc)
              gst_all_1.gst-plugins-bad # fakevideosink
              gst_all_1.gst-plugins-base # appsink
              gst_all_1.gst-plugins-good # autoaudiosink

              # for xdg-open
              xdg-utils
            ];

            shellHook =
              let
                libraryPath = pkgs.lib.makeLibraryPath [ pkgs.libayatana-appindicator ];
              in
              # bash
              ''
                export LD_LIBRARY_PATH="${libraryPath}:$LD_LIBRARY_PATH"
                export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH:$GST_PLUGIN_SYSTEM_PATH_1_0" # Needed on Wayland to report the correct display scale
                # fixes tauri networking problems (like images not loading)
                export GIO_MODULE_DIR="${pkgs.glib-networking}/lib/gio/modules/";
              '';
          };
        }
      );

      packages = eachSystem (
        system:
        let
          version = inputs.self.shortRev or inputs.self.dirtyShortRev or "unknown";

          pkgs = pkgsFor.${system};
          inherit (pkgs) lib;
          craneLib = inputs.crane.mkLib pkgs;

          frontend = pkgs.buildNpmPackage (final: {
            pname = "unchartable-frontend";
            inherit version;

            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./public
                ./src

                ./index.html
                ./package-lock.json
                ./package.json
                ./tsconfig.json
                ./tsconfig.node.json
                ./vite.config.ts
              ];
            };

            npmDeps = pkgs.importNpmLock { npmRoot = final.src; };
            npmConfigHook = pkgs.importNpmLock.npmConfigHook;

            installPhase = ''
              runHook preInstall
              cp -r dist $out
              runHook postInstall
            '';
          });

          tauri = inputs.crane-tauri.lib.buildTauriApp { inherit pkgs craneLib; } {
            pname = "unchartable-app";
            inherit version;
            src = ./.;
            inherit frontend;
          };
        in
        {
          unchartable-app = pkgs.callPackage ./package.nix { inherit (tauri) app; };
        }
      );
    };
}
