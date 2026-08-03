{
  stdenv,
  lib,

  app,

  wrapGAppsHook3,

  libayatana-appindicator,

  glib-networking,
  gst_all_1,
  xdg-utils,
}:
stdenv.mkDerivation (final: {
  pname = "unchartable-app";
  inherit (app) version;

  src = app;

  nativeBuildInputs = lib.optionals stdenv.hostPlatform.isLinux [ wrapGAppsHook3 ];

  # see: https://github.com/tauri-apps/libappindicator-rs/issues/49
  # see: https://nixos.org/manual/nixpkgs/unstable/#ssec-gnome-hooks
  preFixup = ''
    gappsWrapperArgs+=(
      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libayatana-appindicator ]}
    )
  '';

  installPhase = ''
    runHook preInstall

    mkdir -p $out/bin
    cp -r ${app}/bin/${final.pname} $out/bin/

    runHook postInstall
  '';

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    # fixes tauri networking problems (like images not loading)
    glib-networking

    # audio & video playback (track previews, etc)
    gst_all_1.gst-plugins-bad # fakevideosink
    gst_all_1.gst-plugins-base # appsink
    gst_all_1.gst-plugins-good # autoaudiosink

    # for xdg-open which the tauri opener plugin (presumably) uses
    xdg-utils
  ];

  meta = {
    description = "A chart manager for UNBEATABLE";
    homepage = "https://unchartable.site/";
    license = lib.licenses.mit;
    platforms = [
      "x86_64-linux"
    ];
    maintainers = [
      {
        github = "ArtixBTW";
        githubId = 44449514;
      }
    ];
  };
})
