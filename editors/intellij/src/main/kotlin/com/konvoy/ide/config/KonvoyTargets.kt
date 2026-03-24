package com.konvoy.ide.config

import org.jetbrains.kotlin.konan.target.KonanTarget

/**
 * Maps Konvoy target names to Kotlin/Native KonanTarget values.
 * Mirrors the mapping in `crates/konvoy-targets/src/lib.rs`.
 */
object KonvoyTargets {
    private val TARGET_MAP = mapOf(
        "linux_x64" to KonanTarget.LINUX_X64,
        "linux_arm64" to KonanTarget.LINUX_ARM64,
        "macos_x64" to KonanTarget.MACOS_X64,
        "macos_arm64" to KonanTarget.MACOS_ARM64,
    )

    fun toKonanTarget(konvoyTarget: String): KonanTarget? = TARGET_MAP[konvoyTarget]

    fun hostTarget(): KonanTarget {
        val os = System.getProperty("os.name")?.lowercase() ?: ""
        val arch = System.getProperty("os.arch")?.lowercase() ?: ""
        return when {
            os.contains("linux") && arch == "amd64" -> KonanTarget.LINUX_X64
            os.contains("linux") && arch == "aarch64" -> KonanTarget.LINUX_ARM64
            os.contains("mac") && arch == "aarch64" -> KonanTarget.MACOS_ARM64
            os.contains("mac") && arch == "x86_64" -> KonanTarget.MACOS_X64
            // Fallback for Apple Silicon reporting as x86_64 under Rosetta
            os.contains("mac") && arch == "amd64" -> KonanTarget.MACOS_X64
            else -> KonanTarget.MACOS_ARM64 // best-effort fallback
        }
    }

    fun hostTargetName(): String {
        val os = System.getProperty("os.name")?.lowercase() ?: ""
        val arch = System.getProperty("os.arch")?.lowercase() ?: ""
        return when {
            os.contains("linux") && arch == "amd64" -> "linux_x64"
            os.contains("linux") && arch == "aarch64" -> "linux_arm64"
            os.contains("mac") && arch == "aarch64" -> "macos_arm64"
            os.contains("mac") && (arch == "x86_64" || arch == "amd64") -> "macos_x64"
            else -> "macos_arm64"
        }
    }

    /**
     * Convert a Konvoy target name to the Maven suffix format (no underscores).
     * e.g. "linux_x64" -> "linuxx64"
     */
    fun toMavenSuffix(konvoyTarget: String): String = konvoyTarget.replace("_", "")
}
