package com.konvoy.ide.toml

/**
 * Defines the konvoy.toml schema — sections, keys, and valid values.
 * Derived from the Rust `Manifest` struct in `crates/konvoy-config/src/manifest.rs`.
 */
object KonvoyTomlSchema {

    data class KeyInfo(
        val description: String,
        val required: Boolean = false,
        val values: List<String>? = null,
    )

    /** Top-level sections that can appear in konvoy.toml. */
    val SECTIONS = setOf("package", "toolchain", "codegen", "dependencies", "plugins")

    /** Keys within each section. */
    val SECTION_KEYS: Map<String, Map<String, KeyInfo>> = mapOf(
        "package" to mapOf(
            "name" to KeyInfo("Package name", required = true),
            "kind" to KeyInfo("Output kind", values = listOf("bin", "lib")),
            "version" to KeyInfo("Package version (semver)"),
            "entrypoint" to KeyInfo("Entry point file path (default: src/main.kt)"),
        ),
        "toolchain" to mapOf(
            "kotlin" to KeyInfo("Kotlin/Native version", required = true),
            "detekt" to KeyInfo("Detekt linter version"),
        ),
    )

    /** Keys within the OpenAPI codegen sub-table ([codegen.openapi]). */
    val OPENAPI_CODEGEN_KEYS: Map<String, KeyInfo> = mapOf(
        "version" to KeyInfo("Fabrikt version", required = true),
        "spec" to KeyInfo("Project-relative OpenAPI spec path", required = true),
        "base_package" to KeyInfo("Kotlin package for generated sources", required = true),
        "spec_dirs" to KeyInfo(
            "Directories whose files feed the codegen cache key (required; may be empty: spec_dirs = [])",
            required = true,
        ),
    )

    /** Keys within a dependency sub-table (e.g., [dependencies.foo]). */
    val DEPENDENCY_KEYS: Map<String, KeyInfo> = mapOf(
        "path" to KeyInfo("Path to local dependency project"),
        "version" to KeyInfo("Maven dependency version"),
        "maven" to KeyInfo("Maven coordinate (groupId:artifactId)"),
    )

    /** Keys within a plugin sub-table (e.g., [plugins.serialization]). */
    val PLUGIN_KEYS: Map<String, KeyInfo> = mapOf(
        "maven" to KeyInfo("Maven coordinate (groupId:artifactId)", required = true),
        "version" to KeyInfo("Plugin version", required = true),
    )

    /** Returns the valid keys for a given section path like "package" or "dependencies.foo". */
    fun keysForSection(sectionPath: String): Map<String, KeyInfo>? {
        SECTION_KEYS[sectionPath]?.let { return it }
        if (sectionPath == "codegen.openapi") return OPENAPI_CODEGEN_KEYS
        if (sectionPath.startsWith("dependencies.")) return DEPENDENCY_KEYS
        if (sectionPath.startsWith("plugins.")) return PLUGIN_KEYS
        return null
    }
}
