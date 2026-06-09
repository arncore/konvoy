package com.konvoy.ide.config

import com.intellij.testFramework.fixtures.BasePlatformTestCase

/**
 * Tests the real (PSI) manifest/lockfile parsing. These previously called the
 * text-fallback parser directly; that fallback was removed (the PSI path is the
 * only parser), so these now exercise `parseManifest`/`parseLockfile` via a
 * configured TOML PSI file — i.e. what actually runs in the IDE.
 */
class KonvoyTomlParserTest : BasePlatformTestCase() {

    private fun manifest(content: String): KonvoyManifest? {
        val file = myFixture.configureByText("konvoy.toml", content)
        return KonvoyTomlParser.parseManifest(project, file.virtualFile)
    }

    private fun lockfile(content: String): KonvoyLockfile? {
        val file = myFixture.configureByText("konvoy.lock", content)
        return KonvoyTomlParser.parseLockfile(project, file.virtualFile)
    }

    fun testParseMinimalManifest() {
        val manifest = manifest(
            """
            [package]
            name = "hello"

            [toolchain]
            kotlin = "2.3.20"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        assertEquals("hello", manifest!!.`package`.name)
        assertEquals(PackageKind.BIN, manifest.`package`.kind)
        assertEquals("src/main.kt", manifest.`package`.entrypoint)
        assertEquals("2.3.20", manifest.toolchain.kotlin)
        assertNull(manifest.toolchain.detekt)
        assertTrue(manifest.dependencies.isEmpty())
    }

    fun testParseManifestWithAllPackageFields() {
        val manifest = manifest(
            """
            [package]
            name = "mylib"
            kind = "lib"
            version = "1.0.0"
            entrypoint = "src/lib.kt"

            [toolchain]
            kotlin = "2.2.0"
            detekt = "1.23.7"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        assertEquals("mylib", manifest!!.`package`.name)
        assertEquals(PackageKind.LIB, manifest.`package`.kind)
        assertEquals("1.0.0", manifest.`package`.version)
        assertEquals("src/lib.kt", manifest.`package`.entrypoint)
        assertEquals("2.2.0", manifest.toolchain.kotlin)
        assertEquals("1.23.7", manifest.toolchain.detekt)
    }

    fun testParseManifestWithPathDependency() {
        val manifest = manifest(
            """
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"

            [dependencies.mylib]
            path = "../mylib"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        assertEquals(1, manifest!!.dependencies.size)
        val dep = manifest.dependencies["mylib"]
        assertNotNull(dep)
        assertTrue(dep!!.isPath)
        assertEquals("../mylib", dep.path)
    }

    fun testParseManifestWithMavenDependency() {
        val manifest = manifest(
            """
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"

            [dependencies.serialization-core]
            maven = "org.jetbrains.kotlinx:kotlinx-serialization-core"
            version = "1.7.3"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        val dep = manifest!!.dependencies["serialization-core"]
        assertNotNull(dep)
        assertTrue(dep!!.isMaven)
        assertEquals("org.jetbrains.kotlinx:kotlinx-serialization-core", dep.maven)
        assertEquals("1.7.3", dep.version)
    }

    fun testParseManifestWithOpenApiCodegen() {
        val manifest = manifest(
            """
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"

            [codegen.openapi]
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        val openapi = manifest!!.codegen.openapi
        assertNotNull(openapi)
        assertEquals("20.0.0", openapi!!.version)
        assertEquals("specs/api.yaml", openapi.spec)
        assertEquals("com.example.api", openapi.basePackage)
    }

    fun testReturnsNullForMissingPackageSection() {
        assertNull(
            manifest(
                """
                [toolchain]
                kotlin = "2.3.20"
                """.trimIndent(),
            ),
        )
    }

    fun testReturnsNullForMissingToolchainSection() {
        assertNull(
            manifest(
                """
                [package]
                name = "hello"
                """.trimIndent(),
            ),
        )
    }

    fun testReturnsNullForMissingName() {
        assertNull(
            manifest(
                """
                [package]
                kind = "bin"

                [toolchain]
                kotlin = "2.3.20"
                """.trimIndent(),
            ),
        )
    }

    fun testReturnsNullForMissingKotlinVersion() {
        assertNull(
            manifest(
                """
                [package]
                name = "hello"

                [toolchain]
                detekt = "1.23.7"
                """.trimIndent(),
            ),
        )
    }

    fun testParseLockfile() {
        val lock = lockfile(
            """
            [toolchain]
            konanc_version = "2.3.20"
            konanc_tarball_sha256 = "abc123"
            jre_tarball_sha256 = "def456"

            [codegen_tools.fabrikt]
            version = "20.0.0"
            sha256 = "f00d"
            """.trimIndent(),
        )

        assertNotNull(lock)
        assertNotNull(lock!!.toolchain)
        assertEquals("2.3.20", lock.toolchain!!.konancVersion)
        assertEquals("abc123", lock.toolchain!!.konancTarballSha256)
        assertEquals("def456", lock.toolchain!!.jreTarballSha256)
        assertEquals("20.0.0", lock.codegenTools["fabrikt"]!!.version)
        assertEquals("f00d", lock.codegenTools["fabrikt"]!!.sha256)
    }

    fun testIgnoresCommentsAndBlankLines() {
        val manifest = manifest(
            """
            # This is a comment

            [package]
            name = "hello"

            # Another comment
            [toolchain]
            kotlin = "2.3.20"
            """.trimIndent(),
        )

        assertNotNull(manifest)
        assertEquals("hello", manifest!!.`package`.name)
    }

    fun testKindDefaultsToBin() {
        val manifest = manifest(
            """
            [package]
            name = "app"

            [toolchain]
            kotlin = "2.3.20"
            """.trimIndent(),
        )

        assertEquals(PackageKind.BIN, manifest!!.`package`.kind)
    }
}
