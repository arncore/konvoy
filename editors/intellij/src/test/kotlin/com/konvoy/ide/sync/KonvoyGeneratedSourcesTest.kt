package com.konvoy.ide.sync

import junit.framework.TestCase
import java.io.File
import java.nio.file.Files

/**
 * Pure-logic tests for on-disk discovery of generated-source directories. No
 * IntelliJ platform needed.
 */
class KonvoyGeneratedSourcesTest : TestCase() {

    private fun tempProject(): File = Files.createTempDirectory("konvoy-gen-test").toFile()

    fun testNoKonvoyDirIsEmpty() {
        assertTrue(KonvoyGeneratedSources.generatedSourceDirs(tempProject()).isEmpty())
    }

    fun testKonvoyDirWithoutGenIsEmpty() {
        val proj = tempProject()
        File(proj, ".konvoy/build").mkdirs()
        File(proj, ".konvoy/cache").mkdirs()
        assertTrue(KonvoyGeneratedSources.generatedSourceDirs(proj).isEmpty())
    }

    fun testReturnsEachGeneratorDirSortedByName() {
        val proj = tempProject()
        File(proj, ".konvoy/gen/openapi").mkdirs()
        File(proj, ".konvoy/gen/grpc").mkdirs()
        val dirs = KonvoyGeneratedSources.generatedSourceDirs(proj)
        assertEquals(listOf("grpc", "openapi"), dirs.map { it.name })
    }

    fun testIgnoresNonDirectoryEntriesUnderGen() {
        val proj = tempProject()
        File(proj, ".konvoy/gen").mkdirs()
        File(proj, ".konvoy/gen/stray.txt").writeText("not a generator dir")
        File(proj, ".konvoy/gen/openapi").mkdirs()
        val dirs = KonvoyGeneratedSources.generatedSourceDirs(proj)
        assertEquals(listOf("openapi"), dirs.map { it.name })
    }

    fun testEmptyGenDirIsEmpty() {
        val proj = tempProject()
        File(proj, ".konvoy/gen").mkdirs()
        assertTrue(KonvoyGeneratedSources.generatedSourceDirs(proj).isEmpty())
    }
}
