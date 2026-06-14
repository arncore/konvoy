package com.konvoy.ide.sync

import java.io.File

/**
 * Locates a Konvoy project's generated-source output directories.
 *
 * Codegen writes each generator's sources under `.konvoy/gen/<generator>/` (e.g.
 * `.konvoy/gen/openapi/`). These are detected purely on disk — the plugin never
 * parses `[codegen]` config; konvoy owns what is generated and where.
 *
 * The generator *output* directory is returned (not a nested `src/main/kotlin`), so
 * this stays generator-agnostic: Kotlin resolves symbols by their `package`
 * declaration, so the directory layout inside the output dir doesn't matter for
 * indexing.
 */
object KonvoyGeneratedSources {

    /**
     * The generator output dirs under `<projectDir>/.konvoy/gen/` (one per
     * generator), sorted by name. Empty when nothing has been generated yet.
     */
    fun generatedSourceDirs(projectDir: File): List<File> {
        val genDir = File(projectDir, ".konvoy/gen")
        if (!genDir.isDirectory) return emptyList()
        return genDir.listFiles()
            ?.filter { it.isDirectory }
            ?.sortedBy { it.name }
            ?: emptyList()
    }
}
