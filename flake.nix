{
  description = "rust-android-kb: offline multilingual keyboard engine in Rust + a thin Android IME shell";

  inputs = {
    nixpkgs.url = "nixpkgs";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, fenix, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        config = { allowUnfree = true; android_sdk.accept_license = true; };
      };
      ndkVersion = "29.0.14206865";
      android = pkgs.androidenv.composeAndroidPackages {
        platformVersions = [ "35" ];
        buildToolsVersions = [ "37.0.0" ];
        includeNDK = true;
        ndkVersions = [ ndkVersion ];
        includeEmulator = false;
        includeSystemImages = false;
        includeSources = false;
      };
      rust = with fenix.packages.${system}; combine [
        stable.cargo
        stable.rustc
        stable.clippy
        stable.rustfmt
        targets.aarch64-linux-android.stable.rust-std
      ];
      sdk = "${android.androidsdk}/libexec/android-sdk";
    in
    {
      # `nix develop`: everything android/build.sh and the Rust workspace need.
      devShells.${system}.default = pkgs.mkShell {
        packages = [ rust pkgs.cargo-ndk pkgs.jdk17 android.androidsdk pkgs.python3 pkgs.zip ];
        ANDROID_HOME = sdk;
        ANDROID_NDK_HOME = "${sdk}/ndk/${ndkVersion}";
      };
    };
}
