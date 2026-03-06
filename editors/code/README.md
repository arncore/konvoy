# Konvoy for VS Code

VS Code extension for [Konvoy](https://github.com/arncore/konvoy), a native-first Kotlin build tool.

## Install

Install from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=konvoy.konvoy-vscode), or search "Konvoy" in the Extensions panel.

### From source

```
cd editors/code
npm install
npm run build
npx vsce package --no-dependencies
code --install-extension konvoy-vscode-*.vsix
```

## Features

### Commands

All commands are available via `Ctrl+Shift+P`:

| Command | Description |
|---------|-------------|
| Konvoy: Build | Compile the project |
| Konvoy: Build (Release) | Compile in release mode |
| Konvoy: Run | Build and run |
| Konvoy: Run (Release) | Build and run in release mode |
| Konvoy: Test | Run tests |
| Konvoy: Lint | Run detekt linter |
| Konvoy: Update | Resolve Maven dependencies |
| Konvoy: Clean | Remove build artifacts |
| Konvoy: Doctor | Check environment setup |
| Konvoy: Install Toolchain | Install Kotlin/Native |
| Konvoy: List Toolchains | List installed versions |

A run button also appears in the editor title bar when viewing `.kt` files or `konvoy.toml`.

### `konvoy.toml` support

- Syntax highlighting
- Validation on save (mirrors the Rust `Manifest::validate()` rules)
- Autocomplete for section headers, keys, and values
- Hover documentation for all keys
- JSON Schema for Taplo integration

### Diagnostics

Build errors and detekt findings are parsed and shown in the Problems panel. Supported formats:

- `file.kt:10:5: error: message` (konanc)
- `file.kt:3:5: message [RuleName]` (detekt)

### Tasks

`Ctrl+Shift+B` shows auto-detected konvoy tasks (build, test, run, lint, clean, doctor).

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `konvoy.path` | `""` | Path to konvoy binary (empty = use PATH) |
| `konvoy.defaultTarget` | `""` | Default build target |
| `konvoy.showBuildOutputOnSuccess` | `false` | Show output panel even on success |

## Requirements

- [Kotlin Language](https://marketplace.visualstudio.com/items?itemName=fwcd.kotlin) (installed automatically)
- [konvoy](https://github.com/arncore/konvoy) binary on PATH or configured via `konvoy.path`
