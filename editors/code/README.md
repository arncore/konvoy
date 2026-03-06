# Konvoy for VS Code

Build, run, test, and lint [Kotlin/Native](https://kotlinlang.org/docs/native-overview.html) projects powered by [Konvoy](https://github.com/arncore/konvoy) — a native-first Kotlin build tool.

## Features

### Commands

All commands are available via `Ctrl+Shift+P` (or `Cmd+Shift+P` on macOS):

| Command | Description |
|---------|-------------|
| **Konvoy: Build** | Compile the project |
| **Konvoy: Build (Release)** | Compile in release mode |
| **Konvoy: Build...** | Pick build variant (Debug / Release) |
| **Konvoy: Run** | Build and run |
| **Konvoy: Run (Release)** | Build and run in release mode |
| **Konvoy: Toggle Debug/Release** | Switch run variant between debug and release |
| **Konvoy: Test** | Run tests |
| **Konvoy: Lint** | Run detekt linter |
| **Konvoy: Update** | Resolve Maven dependencies |
| **Konvoy: Clean** | Remove build artifacts |
| **Konvoy: Clean All** | Remove all build artifacts (`--all`) |
| **Konvoy: Clean (Confirm)** | Clean with confirmation dialog |
| **Konvoy: Doctor** | Check environment setup |
| **Konvoy: Install Toolchain** | Install Kotlin/Native toolchain |
| **Konvoy: List Toolchains** | List installed toolchain versions |

### Editor title bar

When viewing `.kt` files, `konvoy.toml`, or `konvoy.lock`, the editor title bar shows quick-access buttons for Build, Test, Update, Lint, Clean, and Doctor.

- **Build** — opens a dropdown to pick Debug or Release
- **Run** — toggleable button that swaps between Debug (`$(debug-alt)`) and Release (`$(play)`) via the status bar toggle

### `konvoy.toml` support

- **Syntax highlighting** for `konvoy.toml` manifest files
- **Validation on save** — catches missing fields, invalid plugin configs, bad Maven coordinates, and more
- **Autocomplete** for section headers, keys, and enum values
- **Hover documentation** for all configuration keys
- **JSON Schema** for Taplo integration

### Diagnostics

Build errors and detekt findings are parsed and shown inline in the editor and in the Problems panel:

- `file.kt:10:5: error: message` — konanc compiler errors and warnings
- `file.kt:3:5: message [RuleName]` — detekt lint findings

### Tasks

`Ctrl+Shift+B` (or `Cmd+Shift+B` on macOS) shows auto-detected konvoy tasks: build, test, run, lint, clean, and doctor.

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `konvoy.path` | `""` | Path to the konvoy binary. Leave empty to use `PATH`. |
| `konvoy.defaultTarget` | `""` | Default target platform for builds (e.g. `linux_x64`, `macos_arm64`). |
| `konvoy.showBuildOutputOnSuccess` | `false` | Show the output panel even when the build succeeds. |

## Requirements

- **[Konvoy](https://github.com/arncore/konvoy)** installed and on your `PATH` (or set `konvoy.path` in settings)
- **[Kotlin Language](https://marketplace.visualstudio.com/items?itemName=fwcd.kotlin)** extension — installed automatically as a dependency
