# Installer artwork

The banner and sidebar use the OpenATC navy and cyan branding. `banner.png` is the wide master; `sidebar.png` is the portrait master. The BMP variants are sized for the Windows wizard at normal and double density. The icon comes from the existing `assets/logo.svg`, retaining the established tower identity.

`setup.ico` contains 16, 32, 48, 64, 128 and 256 pixel images. `icon.png` supplies the Mac icon conversion performed by the native packaging job. `flatpak-icon.png` is a pre-rendered 128×128 version of the same tower for Flatpak export, avoiding a build-host SVG decoder dependency. No new image library is required to build packages; derived assets are supplied in the source tree.

The Windows wizard uses native controls and a modern layout. The offline welcome page uses the wide banner with readable dark navy and cyan styling. Linux installation remains script-based; this artwork does not introduce a Linux installer GUI or alter the plugin interface.
