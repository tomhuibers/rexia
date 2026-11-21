{
  lib,
  craneLib,
  fetchzip,
  runCommand,
  zig_0_17,
  pkg-config,
  makeWrapper,
  desktop-file-utils,
  fontconfig,
  freetype,
  libxcb,
  libxkbcommon,
  wayland,
  vulkan-loader,
  libGL,
}:
let
  # ghostty-vt-sys builds libghostty-vt from this source archive with Zig. The
  # crate's GHOSTTY.lock pins the same URL; fetching it here keeps builds offline.
  ghosttySource = fetchzip {
    url = "https://tip.files.ghostty.org/83fb95624b0439ee660c2679b43b38c8b63769cd/libghostty-vt-source.tar.gz";
    hash = "sha256-uRtcTrlsOhUl1lBOwn+I2lc+dP9wDs0G8hxqr4T/5Vk=";
  };
  ghosttyDeps = zig_0_17.fetchDeps {
    pname = "libghostty-vt";
    version = "83fb956";
    src = ghosttySource;
    fetchAll = true;
    hash = "sha256-82Ife6Av0wfZLmovdEdJFgsG5MI4swCeOB3dbvgBDSU=";
  };
  # Zig 0.17 fetches archives, while --system expects extracted package trees.
  ghosttySystemDeps = runCommand "libghostty-vt-system-deps" { } ''
    mkdir -p $out
    for archive in ${ghosttyDeps}/*.tar.gz; do
      tar -xzf "$archive" -C $out
    done
  '';
  runtimeLibraries = [
    fontconfig
    freetype
    libxcb
    libxkbcommon
    wayland
    vulkan-loader
    libGL
  ];
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../assets
      ../resources
      ../vendor
      # The catalog's unit test checks coverage against the requested names.
      ../tests/fixtures/program-icons.txt
    ];
  };

  # Keep real patched dependencies: dummying these would prevent their expensive
  # GPUI/Ghostty builds from being reused by the application derivation.
  dummySrc = craneLib.mkDummySrc {
    inherit src;
    extraDummyScript = ''
      rm -rf "$out/vendor"
      cp -r ${../vendor} "$out/vendor"
    '';
  };

  cargoVendorDir = craneLib.vendorCargoDeps {
    cargoLock = ../Cargo.lock;
    overrideVendorCargoPackage =
      p: drv:
      if p.name == "ghostty-vt-sys" then
        drv.overrideAttrs (old: {
          # Registry tarballs have no Git submodule. Watching a missing path
          # makes Cargo rerun the Zig build every time, defeating artifact reuse.
          postPatch = (old.postPatch or "") + ''
            substituteInPlace build.rs --replace-fail \
              'println!("cargo:rerun-if-changed={}", gitlink.display());' \
              'if gitlink.exists() { println!("cargo:rerun-if-changed={}", gitlink.display()); }'
          '';
        })
      else
        drv;
  };

  commonArgs = {
    inherit src cargoVendorDir;
    pname = "rexia";
    version = (lib.importTOML ../Cargo.toml).package.version;
    strictDeps = true;

    nativeBuildInputs = [
      pkg-config
      makeWrapper
      desktop-file-utils
      zig_0_17
    ];
    buildInputs = runtimeLibraries;

    GHOSTTY_SOURCE_DIR = ghosttySource;
    GHOSTTY_ZIG_SYSTEM_DIR = ghosttySystemDeps;
    dontUseZigBuild = true;
    dontUseZigCheck = true;
    dontUseZigInstall = true;

    preConfigure = ''
      export ZIG_GLOBAL_CACHE_DIR="$TMPDIR/zig-cache"
    '';
  };

  cargoArtifacts = craneLib.buildDepsOnly (
    (builtins.removeAttrs commonArgs [ "src" ])
    // {
      inherit dummySrc;
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;
    # This embeds the final package path in Rexia, not in its dependencies.
    preBuild = ''
      export REXIA_BIND_HELPER="$out/libexec/rexia-bind-port"
    '';

    postInstall = ''
      install -d $out/libexec $out/share/polkit-1/actions
      mv $out/bin/rexia-bind-port $out/libexec/rexia-bind-port
      substitute resources/dev.hackr.rexia.bind-port.policy \
        $out/share/polkit-1/actions/dev.hackr.rexia.bind-port.policy \
        --replace-fail /usr/libexec/rexia-bind-port "$out/libexec/rexia-bind-port"
      desktop-file-install --dir=$out/share/applications \
        --set-key=Exec --set-value=$out/bin/rexia \
        --set-key=TryExec --set-value=$out/bin/rexia \
        resources/dev.hackr.rexia.desktop
      desktop-file-validate $out/share/applications/dev.hackr.rexia.desktop
      install -Dm644 resources/dev.hackr.rexia.svg \
        $out/share/icons/hicolor/scalable/apps/dev.hackr.rexia.svg
      install -Dm644 resources/noctalia/rexia.toml $out/share/rexia/noctalia/rexia.toml
      install -d $out/share/doc/rexia/program-icons
      cp assets/programs/README.md assets/programs/SOURCES.md assets/programs/catalog.json \
        $out/share/doc/rexia/program-icons/
      cp -r assets/programs/licenses $out/share/doc/rexia/program-icons/
      wrapProgram $out/bin/rexia \
        --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibraries}
    '';

    passthru = {
      inherit
        ghosttySource
        ghosttySystemDeps
        runtimeLibraries
        cargoArtifacts
        ;
    };

    meta = {
      description = "Native Linux client for Rex, built on GPUI and themed by Noctalia";
      mainProgram = "rexia";
      platforms = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
  }
)
