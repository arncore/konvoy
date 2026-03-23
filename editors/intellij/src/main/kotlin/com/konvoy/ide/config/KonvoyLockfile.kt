package com.konvoy.ide.config

/**
 * Parsed representation of `konvoy.lock`.
 * Field names match the Rust `Lockfile` struct in `crates/konvoy-config/src/lockfile.rs`.
 */
data class KonvoyLockfile(
    val toolchain: ToolchainLock? = null,
    val dependencies: List<DependencyLock> = emptyList(),
    val plugins: List<PluginLock> = emptyList(),
)

data class ToolchainLock(
    val konancVersion: String,
    val konancTarballSha256: String? = null,
    val jreTarballSha256: String? = null,
    val detektVersion: String? = null,
    val detektJarSha256: String? = null,
)

data class DependencyLock(
    val name: String,
    val source: DepSource,
    val sourceHash: String,
)

sealed class DepSource {
    data class Path(val path: String) : DepSource()
    data class Maven(
        val version: String,
        val maven: String,
        val targets: Map<String, String> = emptyMap(),
        val requiredBy: List<String> = emptyList(),
        val classifier: String? = null,
    ) : DepSource()
}

data class PluginLock(
    val name: String,
    val maven: String,
    val version: String,
    val sha256: String,
    val url: String,
)
