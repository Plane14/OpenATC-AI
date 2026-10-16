# Release packaging and installer appearance

The plugin and AI server remain separate downloads. A successful package build is not proof of installation, upgrade, removal, GPU/audio compatibility or simulator acceptance on that platform.

## Check packaging before the long build

In GitHub Actions, select **Native platform builds**, choose a version and platform, and enable **preflight_only**. This runs the small packaging checks without the full plugin/server/model builds or publication. The platform preflight checks both product layouts, even when the later product selection is narrower.

Choose the branch containing the committed changes in **Use workflow from**. Manual runs upload downloadable artifacts without creating a tag or publishing a release; the version field labels the packages. Start with **preflight_only** enabled, then run again with it disabled for complete packages and the shared Rust test gate. A single platform/product can be selected for a quicker full-package trial. Download outputs from **Artifacts** on the completed run's summary page. Rerunning an older tagged run still tests that tag's original commit, so it will not include later fixes.

Windows jobs use the Inno Setup 6 compiler supplied by the Windows 2022 runner. Both jobs search the machine-wide and current-user Inno Setup 6 directories, verify the compiler exists and pass its absolute path through `ISCC`. The packaging preflight invokes the compiler on both actual installer templates; its output reports the engine version and incompatible directives fail compilation. Executable file-version metadata is not used because the runner compiler reports zero there. They do not depend on downloading an older Chocolatey package. Windows preflight also pins the Visual Studio linker by absolute path and runs native packaging through PowerShell, so Git Bash cannot select its unrelated Unix `link.exe`. Windows AI-server native dependencies use Ninja: the Visual Studio generator can exceed Windows path limits in the nested Vulkan shader-generator build. Platform preflight jobs run independently so one failure does not cancel the other platforms' diagnostic results.

- Windows compiles the actual Inno appearance settings and images into installers with small fixture payloads. It does not execute or install those fixtures.
- macOS generates both application layouts, converts the tower icon with `sips`/`iconutil`, creates DMGs with `hdiutil`, verifies the DMGs and checks the property lists with `plutil`. It does not launch or install those fixtures.
- Each native preflight also runs the focused installer discovery/transaction tests. Linux validates artwork encodings and builds both ZIP layouts with small fixture payloads. It does not validate DEB/RPM/AppImage/Flatpak runtime behaviour.

The normal full build waits for this job to pass. Preflight fixtures are temporary and are not uploaded as releases. Native tools are still required: Linux checks cannot establish that Inno or Apple's packaging tools succeeded.

To run the same check locally on its matching operating system:

```text
cargo xtask release-preflight linux-x64
cargo xtask release-preflight windows-x64
cargo xtask release-preflight macos-intel
cargo xtask release-preflight macos-arm64
```

Only run the command matching the machine. After preflight, run the manual workflow again with **preflight_only** disabled to build the actual packages. Manual runs upload artifacts and do not publish a GitHub release. Existing version-tag publication remains user-controlled; do not create or push tags automatically.

## Shared appearance

`assets/installer/banner.png` and `sidebar.png` follow the supplied forum artwork, with the personal founder/developer line removed. Their derived Windows BMP and ICO assets are prebuilt inputs; packaging does not require an image editor or another image-processing dependency. `packaging/installer/windows-branding.iss` supplies the native modern wizard appearance. It uses `WizardStyle=modern dynamic` and explicit dark-mode branding images to follow the Windows appearance preference (Inno Setup 6.6 or newer). The server installer compiles with this theme in Inno Setup 6.7.3; interactive appearance still needs review. Wizard image lists use comma-separated paths without quotes around individual entries, as required by the compiler.

The offline `Welcome.html` is assembled from `packaging/installer/welcome.html`. The wide banner, cyan accents and dark navy background are shared across ZIP/DMG packages. Plugin and server instructions differ. All page assets are local. The Mac plugin has an installer application offering Install / Update and Remove before folder selection; the server has its own application wrapper. Both use the existing tower logo as the application icon. Finder-launched installer success and error messages are shown as native dialogs rather than relying on an invisible terminal.

These changes do not alter the simulator plugin UI or add a Linux installer GUI. The Mac welcome page is an offline document included on the DMG, not a claim that Finder's disk-image window has a custom background or a complete custom setup wizard.

## Installation and removal implemented

See [INSTALL.md](INSTALL.md) for the end-user guide included at the top of every real package. Linux plugin downloads are now ZIPs with `install.sh` and `uninstall.sh`, backed by the shared Rust installer; no Zenity or GUI installation is required. Linux AI-server formats remain ZIP, AppImage, DEB, RPM and Flatpak. Windows uses branded per-user Inno Setup.exe installers plus ZIP alternatives. macOS uses separate Intel/Apple Silicon DMGs with app wrappers plus portable ZIPs.

`xtask/src/installer.rs` handles standalone installation records, native/Flatpak/Snap Steam roots, configured external libraries and Steam app manifests. It validates X-Plane 12 using the native executable header/architecture plus its simulator log, or a matching X-Plane 12 Steam app manifest when the log is absent. It deduplicates discovered roots and always allows an explicit location. Discovery is bounded, not a disk scan. Standalone installations need one prior launch to create their version log; fresh Steam installations do not. The simulator must be closed before installation. Missing version evidence, inaccessible paths or unusual library arrangements produce an actionable error or require explicit selection.

Transactions check for a running simulator before staging and again before replacement. An exclusive lock blocks simultaneous installers. A complete payload must contain the native plugin and companion engine. Symlinked trees/files and unsafe ownership-manifest paths are refused. Staging occurs beside the target; the old tree moves to `Resources/OpenATC Backups`, outside the plugin loader directory. A failed replacement rename restores the old tree. This is a recoverable filesystem transaction, not a guarantee against disk failure or power loss. A crash can leave a lock or staging/backup directory that needs inspection; the user guide explains recovery.

A SHA-256 ownership manifest distinguishes managed defaults from user files. Updates preserve modified editable TOML/JSON/TXT/MD files and unowned files, remove unchanged obsolete managed files, and keep a backup. Removal retains modified/unowned files and personal data outside the plugin. Legacy/manual installs without ownership metadata are not blindly uninstalled. Older editable overrides are preserved rather than merged, so users must review schema/prompt changes against new defaults. The installer refuses an incomplete extracted payload before replacing the old copy.

The Windows wizard discovers roots through the bundled helper, validates the selected root before file installation, records its managed simulator location and invokes managed removal from Windows Apps. The temporary discovery helper is first in the solid-compressed file list to avoid extracting the entire payload merely to display the destination page. One registered installer manages one simulator location; portable installs support additional copies. A record for another location is refused rather than silently leaving an orphaned plugin.

The Mac plugin app supports explicit installation and removal with native result dialogs. The server app offers Start/Configure and opens the persistent browser dashboard through its launcher. First-run setup and model download/loading progress use that dashboard; Configure opens the same manager rather than a second inference process. Server application removal preserves separate settings/models; stop its process first. Windows includes Start-menu launch/configuration shortcuts. Linux DEB/RPM provide an optional user service without enabling it automatically. Flatpak exposes network and DRI devices; proprietary GPU driver libraries are excluded from the Linux portable dependency copy. Actual GPU access still depends on host drivers, Flatpak extensions and the chosen model backend.

## Linux Vulkan loader dependency

The Linux AI-server job installs `libvulkan-dev` alongside the pinned shader SDK. Its early loader link/run check catches missing `-lvulkan` before the full native dependency build. Installing shader tools and setting `LD_LIBRARY_PATH` alone does not provide the development library to Rust's linker. End-user GPU support still depends on the installed Vulkan driver; portable packaging excludes proprietary driver libraries.

## Intel Mac ONNX dependency build

The Intel Mac server compiles ONNX Runtime with CoreML support. Its build supplies `CMAKE_POLICY_VERSION_MINIMUM=3.5` for older bundled dependencies such as psimd, whose declared policy version is rejected by CMake 4. This uses CMake's external compatibility setting rather than modifying vendor sources. The ONNX cache key includes this build revision. A dependency configuring successfully on Linux is not evidence of a complete Intel Mac build; the native workflow remains the release check.

## Optional signing and notarisation

Unsigned manual test builds remain possible without secrets. Public publisher identity requires the maintainer's certificates; no certificate or private key is committed. The normal native workflow imports optional credentials before expensive model builds, signs native payload files before archiving and signs Windows setup/removal or the Mac app/DMG. Signing failures fail packaging rather than silently producing an unsigned substitute.

For **Windows**, set protected repository/environment secrets `OPENATC_WINDOWS_CERT_PFX_BASE64` and `OPENATC_WINDOWS_CERT_PASSWORD`. The PowerShell import uses the current-user certificate store and exports the selected private-key certificate thumbprint to the packaging step. The isolated hosted runner is discarded after the job. For a local signing workstation, import its certificate yourself and set `OPENATC_WINDOWS_SIGN_CERT_SHA1` to its 40-hex-character thumbprint, with `signtool.exe` available. Native payloads, Setup.exe and the Inno uninstaller use SHA-256 with an RFC3161 timestamp. The temporary signed-uninstaller cache stays outside release outputs. Hardware-backed/cloud signing systems require their own certificate-provider setup; exporting a PFX is not assumed to be available for every publisher certificate.

For **macOS**, set protected secrets `OPENATC_MAC_CERT_P12_BASE64`, `OPENATC_MAC_CERT_PASSWORD` and `OPENATC_MAC_SIGN_IDENTITY` (the Developer ID Application identity). Import occurs into a temporary keychain removed by an always-run cleanup step. Nested Mach-O files and the application use timestamped signatures and hardened runtime; the enclosing app is verified before the DMG is made. The DMG is signed too.

For notarisation add all three secrets `OPENATC_APPLE_ID`, `OPENATC_APPLE_PASSWORD` (app-specific password) and `OPENATC_APPLE_TEAM_ID`. The workflow stores a temporary `openatc-ci` notary profile. Locally, supply `OPENATC_NOTARY_PROFILE` referencing your prepared keychain profile alongside the signing identity. Packaging requires Apple's returned status to be **Accepted**, staples the DMG and validates the ticket. A signed raw portable ZIP does not contain a stapled app/DMG ticket; recommend the notarised DMG for Mac users. The fixture preflight has no signing credentials and tests package structure, not publisher acceptance.

References: [Inno signed uninstaller](https://jrsoftware.org/ishelp/topic_setup_signeduninstaller.htm), [Apple Developer ID](https://developer.apple.com/developer-id/), [Apple notarisation](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).

## Verification and remaining platform acceptance

Local checks on 15 October 2026:

- Focused installer suite: six passing tests for discovery/external Unicode paths, literal Windows path escaping, upgrade/removal preservation, rollback, unsafe manifests, wrong major version/running-process guard, symlinks and incomplete extraction. Earlier full xtask run passed nine tests; the newly added sixth installer regression passes in the focused final run.
- Linux release tooling compiles, artwork/ZIP fixture preflight passes, and a real plugin release ZIP builds after validating all speech resources.
- The actual extracted ZIP's shell wrappers and release installer pass a fresh-install → customised prompt/model → upgrade → removal cycle in a temporary Unicode/spaced X-Plane fixture. Checks include archive checksum, executable modes, native payload presence, backup placement, removed owned binaries and retained custom files. No real simulator directory was modified.
- Local workflow YAML parsing and shell syntax checks pass. This is not a native Inno compilation or Apple tools run.

Evidence: `/tmp/openatc-installer-safety-final.log`, `/tmp/openatc-installer-final-build.log`, `/tmp/openatc-installer-preflight-final.log`, `/tmp/openatc-installer-package-final.log`, `/tmp/openatc-installer-packaged-cycle.log`. Generated local packages may coexist with older formats from previous experiments; current Linux plugin builds produce ZIP only. CI starts with a clean output directory.

The remaining release acceptance is the user's manual **preflight_only** run on Windows/Intel Mac/Apple Silicon, followed by actual package builds and native fresh-install/upgrade/removal/runtime checks. Signing/notarisation requires configured publisher credentials and a real signed run. Linux AI-server DEB/RPM/AppImage/Flatpak install/launch/GPU acceptance also remains distinct from packaging code and historical artifacts. Do not claim these tests were run locally on platforms unavailable here. Do not publish from a preflight fixture or call the full OpenATC backlog complete: generated traffic and the saved flight repairs remain separately tracked.
