package com.konvoy.ide.config

import org.junit.Assert.*
import org.junit.Test

class KonvoyTomlParserTest {

    @Test
    fun `parse minimal manifest`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "hello"

            [toolchain]
            kotlin = "2.3.20"
        """.trimIndent())

        assertNotNull(manifest)
        assertEquals("hello", manifest!!.`package`.name)
        assertEquals(PackageKind.BIN, manifest.`package`.kind)
        assertEquals("src/main.kt", manifest.`package`.entrypoint)
        assertEquals("2.3.20", manifest.toolchain.kotlin)
        assertNull(manifest.toolchain.detekt)
        assertTrue(manifest.dependencies.isEmpty())
    }

    @Test
    fun `parse manifest with all package fields`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "mylib"
            kind = "lib"
            version = "1.0.0"
            entrypoint = "src/lib.kt"

            [toolchain]
            kotlin = "2.2.0"
            detekt = "1.23.7"
        """.trimIndent())

        assertNotNull(manifest)
        assertEquals("mylib", manifest!!.`package`.name)
        assertEquals(PackageKind.LIB, manifest.`package`.kind)
        assertEquals("1.0.0", manifest.`package`.version)
        assertEquals("src/lib.kt", manifest.`package`.entrypoint)
        assertEquals("2.2.0", manifest.toolchain.kotlin)
        assertEquals("1.23.7", manifest.toolchain.detekt)
    }

    @Test
    fun `parse manifest with path dependency`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"

            [dependencies.mylib]
            path = "../mylib"
        """.trimIndent())

        assertNotNull(manifest)
        assertEquals(1, manifest!!.dependencies.size)
        val dep = manifest.dependencies["mylib"]
        assertNotNull(dep)
        assertTrue(dep!!.isPath)
        assertEquals("../mylib", dep.path)
    }

    @Test
    fun `parse manifest with maven dependency`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"

            [dependencies.serialization-core]
            maven = "org.jetbrains.kotlinx:kotlinx-serialization-core"
            version = "1.7.3"
        """.trimIndent())

        assertNotNull(manifest)
        val dep = manifest!!.dependencies["serialization-core"]
        assertNotNull(dep)
        assertTrue(dep!!.isMaven)
        assertEquals("org.jetbrains.kotlinx:kotlinx-serialization-core", dep.maven)
        assertEquals("1.7.3", dep.version)
    }

    @Test
    fun `returns null for missing package section`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [toolchain]
            kotlin = "2.3.20"
        """.trimIndent())

        assertNull(manifest)
    }

    @Test
    fun `returns null for missing toolchain section`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "hello"
        """.trimIndent())

        assertNull(manifest)
    }

    @Test
    fun `returns null for missing name`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            kind = "bin"

            [toolchain]
            kotlin = "2.3.20"
        """.trimIndent())

        assertNull(manifest)
    }

    @Test
    fun `returns null for missing kotlin version`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "hello"

            [toolchain]
            detekt = "1.23.7"
        """.trimIndent())

        assertNull(manifest)
    }

    @Test
    fun `parse lockfile text`() {
        val lockfile = KonvoyTomlParser.parseLockfileFromText("""
            [toolchain]
            konanc_version = "2.3.20"
            konanc_tarball_sha256 = "abc123"
            jre_tarball_sha256 = "def456"
        """.trimIndent())

        assertNotNull(lockfile)
        assertNotNull(lockfile.toolchain)
        assertEquals("2.3.20", lockfile.toolchain!!.konancVersion)
        assertEquals("abc123", lockfile.toolchain!!.konancTarballSha256)
        assertEquals("def456", lockfile.toolchain!!.jreTarballSha256)
    }

    @Test
    fun `ignores comments and blank lines`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            # This is a comment

            [package]
            name = "hello"

            # Another comment
            [toolchain]
            kotlin = "2.3.20"
        """.trimIndent())

        assertNotNull(manifest)
        assertEquals("hello", manifest!!.`package`.name)
    }

    @Test
    fun `kind defaults to BIN`() {
        val manifest = KonvoyTomlParser.parseManifestFromText("""
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"
        """.trimIndent())

        assertEquals(PackageKind.BIN, manifest!!.`package`.kind)
    }
}
