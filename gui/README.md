# CerberusMCP GUI (Dear ImGui)

A native C++17 desktop app for CerberusMCP. It links the Rust core as a
static library (`libcerberus_mcp.a` / `.lib`) via the C ABI in
`src/ffi.rs` — the same registry, gateway, doctor, and logs the CLI uses.

## Layout

| Path | Purpose |
| --- | --- |
| `src/app.cpp` | Screens, navigation, dialogs, loader overlay, polling |
| `src/cerberus_api.h/.cpp` | `core::` C-ABI wrappers (init, registry, gateway, doctor, logs) |
| `src/registry.h/.cpp` | Registry model: refresh/save/delete + accent hue |
| `src/config.h/.cpp` | Persisted window/UI preferences (ini) |
| `src/lang/` | Locales: TR, EN, DE, FR, ES, RU, PT |
| `src/ui/` | Widgets, theme, fonts, icons, background scenes |
| `src/host_win32.cpp` | Windows: D3D11 + Win32 window |
| `src/host_glfw.cpp` | macOS/Linux: GLFW + OpenGL3 |
| `packaging/` | Info.plist template for the macOS `.app` |
| `scripts/package-macos.sh` | Build `.app` + `.dmg` from a build dir |

## Build

Requirements: Rust stable, CMake ≥ 3.20, a C++17 compiler, GLFW 3.3
(`brew install glfw` / `apt install libglfw3-dev`), OpenGL headers.

```sh
cd gui
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build --target CerberusMCP
./build/CerberusMCP
```

CMake first builds the Rust static core (`cerberus_core` target), then
links it with the C++ sources and Dear ImGui.

## Packaging

macOS:

```sh
bash scripts/package-macos.sh build 0.4.1
# -> CerberusMCP.app + CerberusMCP_0.4.1_macos.dmg
```

Windows / Linux release artifacts stage the binary next to `assets/`
(see `.github/workflows/release.yml`).
