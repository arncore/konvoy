package com.konvoy.ide.config

/**
 * Parsed representation of `konvoy.toml`.
 * Field names match the Rust `Manifest` struct in `crates/konvoy-config/src/manifest.rs`.
 */
data class KonvoyManifest(
    val `package`: KonvoyPackage,
    val toolchain: KonvoyToolchain,
    val dependencies: Map<String, DependencySpec> = emptyMap(),
    val plugins: Map<String, DependencySpec> = emptyMap(),
)

data class KonvoyPackage(
    val name: String,
    val kind: PackageKind = PackageKind.BIN,
    val version: String? = null,
    val entrypoint: String = "src/main.kt",
)

enum class PackageKind {
    BIN,
    LIB;

    companion object {
        fun fromString(value: String): PackageKind = when (value.lowercase()) {
            "lib" -> LIB
            else -> BIN
        }
    }
}

data class KonvoyToolchain(
    val kotlin: String,
    val detekt: String? = null,
)

data class DependencySpec(
    val path: String? = null,
    val version: String? = null,
    val maven: String? = null,
) {
    val isPath: Boolean get() = path != null
    val isMaven: Boolean get() = maven != null && version != null
}
