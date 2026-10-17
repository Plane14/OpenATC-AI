# Third-party components

## Local speech and model inference

Kitten Mini 0.8 integration adapts the token vocabulary, voice styles and speed priors from [KittenML's KittenTTS implementation](https://github.com/KittenML/KittenTTS/tree/0.8.1). The original is Apache-2.0 licensed; its license is retained in `assets/licenses/KittenTTS-APACHE-2.0.txt`. OpenATC implements inference and chunking in Rust and downloads the pinned ONNX model, voice styles and configuration listed in `models-kitten.toml`. Kitten Mini 0.8 is separate from KittenTTS2 and its licensing.

Kokoro uses the [Kokoro-82M model](https://huggingface.co/hexgrad/Kokoro-82M) and the ONNX/voice exports identified in `models.toml`. Kitten Mini and Kokoro models use Apache-2.0 terms. ONNX Runtime is MIT-licensed. The eSpeak-ng phonemizer is a native GPL-3.0-or-later dependency; its redistribution/source obligations still apply through Rust bindings. The application license does not replace dependency licenses.

Accelerated speech uses separately installed NVIDIA CUDA and cuDNN runtime libraries. These are NVIDIA-licensed components, not MIT application code or downloaded model weights. OpenATC's release package contains its ONNX provider libraries; users supply compatible GPU drivers and NVIDIA runtime libraries. Keep any runtime redistribution within the vendor's terms. Apple and Vulkan backends similarly depend on their platform runtimes.

Windows packaging copies Microsoft C++ x64 release CRT and OpenMP DLLs from the build machine’s licensed Visual Studio redistributable directories. These Microsoft components retain their own redistribution terms, separate from the application’s MIT license; see `assets/licenses/Microsoft-runtime-NOTICE.txt`. Debug runtimes, system-directory DLLs and Visual Studio tools are not bundled. Runtime binaries are generated package contents, not repository assets.

The Windows AI package includes the Vulkan loader so a missing system loader does not prevent CPU fallback from starting. Its upstream Apache-2.0 and permissive notices are retained in `assets/licenses/Vulkan-Loader-LICENSE.txt`; the pinned LunarG runtime archive notice is retained in `assets/licenses/VulkanRT-LICENSE.txt`. CI downloads and checksum-verifies the x64 runtime; its DLL and archive are not repository files. A loader does not supply a hardware driver's Vulkan implementation.

The AI-server package includes this documentation and the project license notices. No model weights or personal provider keys are bundled. See [AI server setup](AI_SERVER.md) for the build features and required runtime files.

The active runtime is the Rust workspace; exact dependency versions are pinned in `Cargo.lock`. It uses imgui/glow for UI, miniaudio behind a native FFI adapter for audio, Axum/reqwest for the companion engine, and xplane/xplane-sys with the SDK 4.0 feature for X-Plane 12 weather APIs. Legacy CMake build files have been removed. Rust dependencies can still contain native C/C++ implementations (including ImGui and miniaudio); first-party Rust does not mean a purely Rust dependency tree. Preserve dependency and SDK redistribution terms when packaging.

Bundled DejaVu Sans and DejaVu Sans Bold fonts retain their license in `assets/fonts/LICENSE.txt`. The OpenATC AI vector logo is project artwork. Font bytes are embedded at build time; no font download or user-system font path is required at runtime.

No X-Plane airport/navdata, Navigraph charts, AIRAC databases, model weights or provider API keys are redistributed. The simulator data readers use the user's installed files. Optional online planning weather is NOAA Aviation Weather Center data. Live ATIS/controller weather instead comes from airport-location X-Plane surface samples. SimBrief and Navigraph access remain subject to their service terms.

The Rust airport map uses earcutr 0.4.3 (ISC) to triangulate installed pavement outlines while preserving holes. Its notice is bundled in `assets/licenses/earcutr-LICENSE`. The illuminated taxi arrow model and textures are original project assets.

The Rust plugin vendors xplane 0.1.0-alpha.1 (MPL 2.0), with a corrected SDK dataref type-mask check. The installer includes its complete source in `source/xplane` and its license in `assets/licenses/xplane-MPL-2.0.txt`. Preserve these files when redistributing the plugin.

The Rust companion engine uses reqwest with default features disabled and rustls/webpki roots for HTTPS. It does not dynamically link OpenSSL; preserve the Rust TLS dependencies’ license notices when distributing.

The Rust UI uses arboard (MIT/Apache-2.0) for native text clipboard access. Linux uses the Rust x11rb transport without linking Xlib into the plugin. Clipboard ownership lives until the UI closes.

## GeoNames settlement data

`assets/geography/places.tsv` is derived from the GeoNames cities1000 download (8 October 2026). Geographical data © GeoNames, https://www.geonames.org/, under CC BY 4.0, https://creativecommons.org/licenses/by/4.0/. Reduced to ASCII names, coordinates, country codes and population; see the asset README for coverage. This dataset is separate from the MIT application license.

The vendored `xplane-sys` 4.0.109 bindings retain their upstream MIT/Apache-2.0 licenses. Their build script corrects the macOS framework search directive; the SDK files are unchanged.

The maintained SDK binding patch generates only XPLM/XP functions, excluding Windows CRT declarations from the SDK's `C-unwind` ABI. The plugin launches its Windows companion as a detached process and loads OpenGL from the host's native library; Windows extension symbols use `wglGetProcAddress`. Windows engine configuration and logs default to `%LOCALAPPDATA%/openatc`, with `OPENATC_CONFIG_DIR` available as an override.

## macOS compatibility drawing

The Rust OpenGL 2.1 drawing path follows the fixed-pipeline approach of Dear ImGui’s [OpenGL 2 backend at v1.89.2](https://github.com/ocornut/imgui/blob/v1.89.2/backends/imgui_impl_opengl2.cpp), with per-command vertex offsets and saved host/client/matrix state. Its MIT notice is retained in `assets/licenses/imgui-opengl2-LICENSE.txt`. OpenGL function bindings are generated during the build by the pinned `gl_generator` crate; generated bindings stay in build output. The compatibility renderer is implemented in Rust.
