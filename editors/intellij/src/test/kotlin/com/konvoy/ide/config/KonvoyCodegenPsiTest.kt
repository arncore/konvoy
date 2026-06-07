package com.konvoy.ide.config

import com.intellij.testFramework.fixtures.BasePlatformTestCase

/**
 * Exercises the PSI-backed parsing path (`parseManifestFromPsi` /
 * `parseLockfileFromPsi`) for codegen, which the text-fallback unit tests do
 * not cover.
 */
class KonvoyCodegenPsiTest : BasePlatformTestCase() {

    fun testParsesOpenApiCodegenViaPsi() {
        val file = myFixture.configureByText(
            "konvoy.toml",
            """
            [package]
            name = "my-app"

            [toolchain]
            kotlin = "2.1.0"

            [codegen.openapi]
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )

        val manifest = KonvoyTomlParser.parseManifest(project, file.virtualFile)
        assertNotNull("manifest should parse via PSI", manifest)
        val openapi = manifest!!.codegen.openapi
        assertNotNull("openapi codegen should be present", openapi)
        assertEquals("20.0.0", openapi!!.version)
        assertEquals("specs/api.yaml", openapi.spec)
        assertEquals("com.example.api", openapi.basePackage)
    }

    fun testManifestWithoutCodegenHasNoOpenApi() {
        val file = myFixture.configureByText(
            "konvoy.toml",
            """
            [package]
            name = "my-app"

            [toolchain]
            kotlin = "2.1.0"
            """.trimIndent(),
        )

        val manifest = KonvoyTomlParser.parseManifest(project, file.virtualFile)
        assertNotNull(manifest)
        assertNull(manifest!!.codegen.openapi)
    }

    fun testParsesCodegenToolsLockViaPsi() {
        val file = myFixture.configureByText(
            "konvoy.lock",
            """
            [toolchain]
            konanc_version = "2.1.0"

            [codegen_tools.fabrikt]
            version = "20.0.0"
            sha256 = "abc123"
            """.trimIndent(),
        )

        val lock = KonvoyTomlParser.parseLockfile(project, file.virtualFile)
        assertNotNull("lockfile should parse via PSI", lock)
        val pin = lock!!.codegenTools["fabrikt"]
        assertNotNull("fabrikt codegen tool pin should be present", pin)
        assertEquals("20.0.0", pin!!.version)
        assertEquals("abc123", pin.sha256)
    }

    fun testKonvoyLockIsRecognizedAsToml() {
        // The .lock extension isn't auto-associated with TOML; the plugin adds
        // the association so the lockfile parses via PSI (and array tables work).
        val file = myFixture.configureByText("konvoy.lock", "[toolchain]\nkonanc_version = \"2.2.0\"\n")
        assertTrue(
            "konvoy.lock must be parsed as a TOML file, was ${file.javaClass.simpleName}",
            file is org.toml.lang.psi.TomlFile,
        )
    }

    fun testParsesMavenDependencyLockViaPsi() {
        val file = myFixture.configureByText(
            "konvoy.lock",
            """
            [toolchain]
            konanc_version = "2.2.0"

            [[dependencies]]
            name = "kotlinx-serialization-core"
            source_type = "maven"
            version = "1.7.3"
            maven = "org.jetbrains.kotlinx:kotlinx-serialization-core"
            source_hash = "abc"

            [dependencies.targets]
            macos_arm64 = "deadbeef"

            [[dependencies]]
            name = "kotlinx-serialization-json"
            source_type = "maven"
            version = "1.7.3"
            maven = "org.jetbrains.kotlinx:kotlinx-serialization-json"
            source_hash = "def"

            [dependencies.targets]
            macos_arm64 = "cafebabe"
            """.trimIndent(),
        )

        val lock = KonvoyTomlParser.parseLockfile(project, file.virtualFile)
        assertNotNull("lockfile should parse via PSI", lock)
        assertEquals("expected 2 maven dependencies", 2, lock!!.dependencies.size)

        val core = lock.dependencies.first { it.name == "kotlinx-serialization-core" }
        val src = core.source
        assertTrue("expected a Maven source, got $src", src is DepSource.Maven)
        src as DepSource.Maven
        assertEquals("1.7.3", src.version)
        assertEquals("org.jetbrains.kotlinx:kotlinx-serialization-core", src.maven)
    }
}
