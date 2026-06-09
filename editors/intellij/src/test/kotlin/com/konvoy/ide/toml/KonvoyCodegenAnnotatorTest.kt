package com.konvoy.ide.toml

import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.testFramework.fixtures.BasePlatformTestCase

/**
 * Verifies the `[codegen.openapi]` validation in [KonvoyTomlAnnotator], which
 * must agree with the Rust manifest validator and the VS Code extension.
 */
class KonvoyCodegenAnnotatorTest : BasePlatformTestCase() {

    private fun errorsFor(codegenBody: String): List<String> {
        val text = """
            [package]
            name = "my-app"

            [toolchain]
            kotlin = "2.1.0"

            [codegen.openapi]
            $codegenBody
        """.trimIndent()
        myFixture.configureByText("konvoy.toml", text)
        return myFixture.doHighlighting()
            .filter { it.severity == HighlightSeverity.ERROR }
            .mapNotNull { it.description }
    }

    private fun assertAnyError(errors: List<String>, substring: String) {
        assertTrue(
            "expected an error containing \"$substring\", got: $errors",
            errors.any { it.contains(substring) },
        )
    }

    private fun warningsFor(codegenBody: String): List<String> {
        val text = """
            [package]
            name = "my-app"

            [toolchain]
            kotlin = "2.1.0"

            [codegen.openapi]
            $codegenBody
        """.trimIndent()
        myFixture.configureByText("konvoy.toml", text)
        return myFixture.doHighlighting()
            .filter { it.severity == HighlightSeverity.WARNING }
            .mapNotNull { it.description }
    }

    fun testValidOpenApiCodegenHasNoErrors() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            spec_dirs = []
            """.trimIndent(),
        )
        assertTrue("expected no errors, got: $errors", errors.isEmpty())
    }

    fun testMissingRequiredKeysFlagged() {
        val errors = errorsFor("version = \"20.0.0\"")
        assertAnyError(errors, "\"spec\"")
        assertAnyError(errors, "\"base_package\"")
        // spec_dirs is required too (may be empty), so its absence is flagged.
        assertAnyError(errors, "\"spec_dirs\"")
    }

    fun testMissingSpecDirsFlagged() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, "\"spec_dirs\"")
    }

    fun testSpecDirsNotFlaggedAsUnknownKey() {
        // spec_dirs is a known key; it must not produce an "Unknown key" warning.
        val warnings = warningsFor(
            """
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            spec_dirs = ["specs"]
            """.trimIndent(),
        )
        assertFalse(
            "spec_dirs must not be flagged as unknown, got: $warnings",
            warnings.any { it.contains("spec_dirs") },
        )
    }

    fun testVersionBelowFloorFlagged() {
        val errors = errorsFor(
            """
            version = "17.0.0"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, "18.0.0 or newer")
    }

    fun testNonNumericVersionFlagged() {
        val errors = errorsFor(
            """
            version = "latest"
            spec = "specs/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, "valid Fabrikt version")
    }

    fun testAbsoluteSpecFlagged() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "/tmp/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, "relative path inside the project")
    }

    fun testParentTraversalSpecFlagged() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "../outside/api.yaml"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, "..")
    }

    fun testUnsupportedSpecExtensionFlagged() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "specs/api.txt"
            base_package = "com.example.api"
            """.trimIndent(),
        )
        assertAnyError(errors, ".yaml")
    }

    fun testInvalidBasePackageFlagged() {
        val errors = errorsFor(
            """
            version = "20.0.0"
            spec = "specs/api.yaml"
            base_package = "com..example"
            """.trimIndent(),
        )
        assertAnyError(errors, "dot-separated identifiers")
    }
}
