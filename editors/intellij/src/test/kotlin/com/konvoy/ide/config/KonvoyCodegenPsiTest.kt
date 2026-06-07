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
}
