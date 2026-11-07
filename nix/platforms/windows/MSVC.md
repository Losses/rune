# Portable MSVC and Windows SDK

`default.nix` and `windows.nix` default to the pinned toolchain in
`msvc-packages.nix`: MSVC **14.44.35207** (compiler servicing version **14.44.35229**, VS 17.14)
and Windows SDK **10.0.26100.0**. Both x64 and ARM64 targets use x64 host
tools. Windows on ARM can run these host tools through x64 emulation.

From the repository root on Windows:

```powershell
C:\nix\bin\nova-nix.exe build windows.nix -A packages.msvc
```

For nova-nix versions without attribute selection, use a wrapper file:

```nix
# msvc-attr.nix, at the repository root
(import ./nix/platforms/windows/default.nix {}).packages.msvc
```

Then run `nova-nix build msvc-attr.nix`. No channel access or downloads occur
during evaluation. Each payload is a separate fixed-output `fetchurl` input.

The builder extracts `Contents/` from VSIX ZIP archives. For SDK MSIs it opens
the Windows Installer database read-only, expands the cabinets with Windows'
`expand.exe`, and maps `File` through `Component` and `Directory` to its final
path. This is the file-extraction equivalent of `msiexec /a`, without executing
MSI actions, registering products, or requiring elevation. Temporary extraction
files live inside the output and are removed on success. Cabinet subprocesses
have a two-minute timeout. The resulting root contains `VC/Tools/MSVC/<version>`
and `Windows Kits/10`; the build checks required tools, headers and libraries
for both target architectures before succeeding.

The derivation uses tab-separated payload records: nova-nix's Windows store-path
rewriting produced unescaped backslashes when paths were embedded in JSON
(`ConvertFrom-Json: Unrecognized escape sequence`). SDK fetches have explicit
store-safe names because URL-derived names contain `%20`, which Nix rejects.

## Selecting another installation

Import the platform with explicit arguments (put this expression in a wrapper
file when using nova-nix):

```nix
import ./nix/platforms/windows/default.nix {
  useSystemMsvc = true;
  vcVersion = "14.39.33519";
  sdkVersion = "10.0.22621.0";
}
```

Alternatively supply `customMsvcRoot`, `vcVersion` and `sdkVersion` for another
already assembled portable tree. A Visual Studio offline download layout is
not an assembled tree. A custom root takes precedence over `useSystemMsvc`.
The default portable root uses the generated versions. The explicit system
option retains the previous versions unless overridden.

`msvc.env` supplies the same variables as `devShells.default`: prepend its
`PATH`, and set `INCLUDE`, `LIB`, `VCINSTALLDIR`, `WindowsSdkDir`,
`WindowsSDKVersion`, `CC`, `CXX`, and `AR`. `msvc.setupHookPwsh` activates the
portable paths, and `devShells.default.shellHookPwsh` also activates Rust,
Flutter, CMake and Ninja. Realize those packages before running the hook. When
activating values printed by `nova-nix eval` manually, translate the logical
`/nix/store/` prefix to `C:/nix/store/`; builders perform that translation automatically.

## Regenerating pins

The PowerShell generator needs only Windows PowerShell, curl, the Windows
Installer COM API, and .NET ZIP support. Run it explicitly; it is never invoked
by evaluation or a build:

```powershell
powershell -NoProfile -File nix/platforms/windows/generate-msvc.ps1 `
  -Toolset 14.44 -Cache C:\rune-ci\win1\manifest-cache
```

Omit `-Toolset` to select the newest VS 2022 toolset (minimum 14.41). The newest
Windows 11 SDK in that manifest is selected automatically. Compiler directory
versions come from the verified VSIX, not the package servicing version. The
selection contains English compiler resources, desktop CRT/STL (including the
companion Store archives that supply `oldnames.lib` and `msvcrt.lib`), and SDK
headers, libraries and resource tools. ATL headers and libraries are included because the prebuilt
DXC library used by the hub needs `atls.lib`; `msvc.nix` exposes their standard
`atlmfc/include` and `atlmfc/lib/<arch>` paths. MFC, Spectre libraries, IDE and
installer engines are excluded. The SDK's common UCRT/MSI cabinets also contain
some x86 files. Required external cabinets are read from each selected MSI's
[Media table](https://learn.microsoft.com/en-us/windows/win32/msi/media-table).

The generator verifies the channel's manifest hash by default, and always
verifies downloaded compiler/MSI payloads against the manifest. Every emitted
payload carries Microsoft's published SHA256; no locally computed payload
hashes replace them.

On 2026-11-07 the channel advertised manifest SHA256
`4cc372d79f02e331c999a926b833a49eb74b30357d30f41ba9cffa1dc0369379`,
but its URL returned 17,954,755 bytes with SHA256
`963760b01f621ed4fa40d1cf97aeb042f7bc6e04e3176c13c38abb19d265f789`
on both Linux and Windows. The default generator correctly rejected this with
`SHA256 mismatch: VisualStudio.vsman`. The checked-in list was generated using
this explicit manifest-only override:

```powershell
powershell -NoProfile -File nix/platforms/windows/generate-msvc.ps1 `
  -Toolset 14.44 -Cache C:\rune-ci\win1\manifest-cache `
  -ManifestSha256 963760b01f621ed4fa40d1cf97aeb042f7bc6e04e3176c13c38abb19d265f789
```

Both digests and the immutable manifest URL are recorded in the generated file.
The override does not bypass hashing or change any payload hash. Review a future
manifest discrepancy before supplying a replacement digest.

## Smoke test

After setting the environment variables above, run:

```powershell
powershell -NoProfile -File nix/platforms/windows/test-msvc.ps1 `
  -WorkDirectory C:\rune-ci\win1\smoke-x64
```

It compiles and runs a C++ program using the STL, UCRT and Windows SDK. On x64 it
also requires `__std_search_1`, `__std_remove_8`, and `__std_replace_8`, reproducing
the static-DXC link requirement. Activate `msvc.env` for `arch = "arm64"` and run
with `-CompileOnly` to check cross-linking without executing an ARM64 binary.
The compiler and executable are run with timeouts.
