# Windows Flutter package boundary

## Delivery acceptance

Windows delivery means the full Release application directory, the reference build workflow MSIX package, and the direct-install Inno package. A standalone Rust DLL is not a deliverable. Artifact names include the sanitized ref, seven-character commit, and target architecture. Missing products fail the workflow. MSIX validation checks manifest architecture, executable/native library PE architecture, and Flutter assets before upload. The Store-configured MSIX follows the reference workflow; Inno is the separate direct-install package.

Flutter 3.47.6 selects the Windows target from the Dart runtime ABI, not a target-platform switch. ARM64 therefore uses a native Windows ARM runner and native Dart; running x64 Dart through emulation is not sufficient. ARM64 implementation and runtime validation are in progress; previous successful DLL-only runs do not satisfy this acceptance.

## SDK layout

The original raw derivation retains the known-working cmd.exe mkdir/tar extraction, unchanged name, inputs, and flutter directory layout so its existing Store result can be reused. A separate small wrapper derivation checks the bundled Dart executable and Flutter snapshot and installs launchers plus sdk-path.txt referencing the raw SDK. It contains no SDK copy or junction for nova-nix output copying to traverse. No archive fixtures are excluded. Neither derivation runs the upstream batch bootstrap or rebuilds the bundled snapshot.

The launcher uses FLUTTER_ROOT to select a per-Store-output facade in LOCALAPPDATA/Rune/Flutter. Flutter 3.47.6 hardcodes both cache lookup and lock acquisition below this root; there is no assumed cache environment override. The facade copies bin/cache except dart-sdk, and packages/flutter_tools (whose PubDependencies artifact may regenerate package configuration). All other SDK directories, including Dart and repository metadata, remain junctions to immutable package content. Root metadata and launcher files are small writable copies. No hardlinks or Store attribute changes are used.

This deliberately duplicates the mutable engine/artifact cache, not the full SDK. Linking engine directories would let artifact replacement write through into the Store. Runtime artifact and Pub downloads remain possible: this is immutable SDK integration, not a fully offline/hermetic artifact closure. The copied subset size depends on the official archive.

Initialization is serialized with a two-minute exclusive-file-lock deadline; the ready marker is atomically renamed and checked against the source identity. Incomplete initialization fails closed and requires manually renaming the incomplete facade; automatic recursive deletion could traverse junctions. Upgrades, downgrades, and channel switching must happen through Nix. The conservative token guard also rejects these words as ordinary Flutter positional arguments; Dart arguments are unaffected. GIT_OPTIONAL_LOCKS=0 suppresses optional repository refresh writes. This is not a security boundary against user code intentionally writing to the SDK.

CI runs bounded, monitored version, repeated nested-launcher, Dart, and Windows precache checks before Rust realization. It compares immutable SDK file metadata before/after (not cryptographic content hashes), and checks cache/Dart link boundaries. Actual Windows execution remains required; Linux Nix parsing alone does not validate PowerShell or junction semantics.

Upstream references researched with gh:
- https://github.com/flutter/flutter/blob/3.47.6/packages/flutter_tools/lib/src/cache.dart
- https://github.com/flutter/flutter/blob/3.47.6/packages/flutter_tools/lib/src/flutter_cache.dart
- https://github.com/NixOS/nixpkgs/blob/master/pkgs/development/compilers/flutter/versions/3_47/patches/disable-auto-update.patch
- https://github.com/NixOS/nixpkgs/blob/master/pkgs/development/compilers/flutter/versions/3_47/patches/deregister-pub-dependencies-artifact.patch

Nixpkgs instead patches the tool and preassembles artifacts. This Windows package retains the official snapshot and its normal cache locking/updating, isolated in the facade, to avoid adding a source snapshot/dependency rebuild pipeline.
