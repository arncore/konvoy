# Konvoy for IntelliJ IDEA

Kotlin/Native project support for [Konvoy](https://github.com/arncore/konvoy) build tool.

## Install

**From JetBrains Marketplace:**

Search for "Konvoy" in **Settings → Plugins → Marketplace**, or install from the [plugin page](https://plugins.jetbrains.com/plugin/com.konvoy.intellij).

**From GitHub Release:**

Download the latest `konvoy-intellij-*.zip` from [Releases](https://github.com/arncore/konvoy/releases), then install via **Settings → Plugins → ⚙️ → Install Plugin from Disk...**.

**From source:**

```bash
cd editors/intellij
./gradlew buildPlugin
# Plugin zip: build/distributions/konvoy-intellij-<version>.zip
```

## Requirements

- IntelliJ IDEA 2024.2+ (Community or Ultimate)
- Kotlin plugin (bundled with IntelliJ)
- Konvoy CLI installed (`konvoy` on PATH)

## Features

### Project sync

Opens any directory with a `konvoy.toml` as a Konvoy project. The plugin reads `konvoy.toml` and `konvoy.lock`, then configures IntelliJ's project model so the Kotlin plugin provides full Kotlin/Native intelligence — no Gradle required.

- `src/` as source root, `src/test/` as test root
- `.konvoy/` excluded from indexing
- klib dependencies from `~/.konvoy/cache/maven/` as project libraries
- Path dependencies with their `src/` as source roots
- Kotlin facet with Native target platform and correct language version
- Compiler plugin classpaths from locked plugins

### Auto re-sync

Watches `konvoy.toml` and `konvoy.lock` for changes and automatically re-syncs the project model with a 500ms debounce. You can also manually sync via **Build → Sync Konvoy Project**.

### Run configurations

Run configurations for all Konvoy commands:

| Configuration | Command |
|--------------|---------|
| Build | `konvoy build` |
| Run | `konvoy run` |
| Test | `konvoy test` |
| Lint | `konvoy lint` |

Right-click on `konvoy.toml` to create a run configuration, or use the run menu.

### Toolchain SDK

Discovers Konvoy-managed toolchains from `~/.konvoy/toolchains/` and registers them as IntelliJ SDKs. If the Kotlin version specified in `konvoy.toml` doesn't have an installed toolchain, the plugin shows a notification with instructions to install it.

### Code intelligence

Once the project model is configured, all Kotlin intelligence comes from IntelliJ's built-in Kotlin plugin:

- Code completion
- Go to definition / find usages
- Rename refactoring
- Error diagnostics
- Quick fixes

## Known limitations

- **K2 mode symbol resolution** — In IntelliJ 2025.3 with K2 mode enabled, stdlib symbols like `println` may show as unresolved. This is due to an upstream bug in the Kotlin plugin's `LibraryEffectiveKindProvider` which ignores `LibraryEntity.typeId` for non-Gradle libraries. A [fix has been submitted](https://github.com/JetBrains/intellij-community/pull/3469) upstream. The "Kotlin is not configured" banner and all other plugin features work correctly.

## Development

```bash
# Build
./gradlew buildPlugin

# Test
./gradlew test

# Run in sandbox IDE
./gradlew runIde
```
