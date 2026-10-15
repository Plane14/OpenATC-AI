# Installer artwork

The banner and sidebar match the supplied OpenATC forum artwork, without its personal signature line. `banner.png` is the wide master; `sidebar.png` is the portrait master. The BMP variants are sized for the Windows wizard at normal and double density. The icon comes from the existing `assets/logo.svg`, retaining the established tower identity.

`setup.ico` contains 16, 32, 48, 64, 128 and 256 pixel images. `icon.png` supplies the Mac icon conversion performed by the native packaging job. No new image library is required to build packages; derived assets are supplied in the source tree.

The Windows wizard uses native controls and a modern layout. The offline welcome page uses the wide banner with readable dark navy and cyan styling. Linux installation remains script-based; this artwork does not introduce a Linux installer GUI or alter the plugin interface.

## Artwork prompts

Created with the built-in image-generation tool, using `/home/bill/Pictures/opatc-forum-footer.png` as the reference. The original forum image was not overwritten.

- Banner: remove the entire “BIL4X4 — Founder & Lead Developer” line; restore matching radar/cloud scenery; preserve the OpenATC AI wordmark, tower, radar rings, route dots, airliner, horizon, tagline and cyan border in the wide composition. No extra text or installer mockup.
- Sidebar: adapt the same brand to a portrait 164:314 composition; retain the navy sky, cyan radar/tower, airliner and warm horizon; keep “OpenATC AI” legible near the top with safe margins. No personal signature, long tagline, extra text or UI mockup.

Windows compilation and Mac application/DMG appearance must still be checked on their native platforms. An image preview or Linux packaging-tool check does not establish that acceptance.
