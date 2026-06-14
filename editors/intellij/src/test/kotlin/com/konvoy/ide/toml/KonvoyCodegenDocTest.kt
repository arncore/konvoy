package com.konvoy.ide.toml

import com.intellij.psi.PsiFile
import com.intellij.psi.util.PsiTreeUtil
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import org.toml.lang.psi.TomlKey

/**
 * Verifies hover documentation for `[codegen.openapi]` — both its keys (from the
 * schema) and the section header.
 */
class KonvoyCodegenDocTest : BasePlatformTestCase() {

    private val sample = """
        [codegen.openapi]
        version = "20.0.0"
        spec = "specs/api.yaml"
        base_package = "com.example.api"
    """.trimIndent()

    private fun key(file: PsiFile, predicate: (TomlKey) -> Boolean): TomlKey =
        PsiTreeUtil.findChildrenOfType(file, TomlKey::class.java).first(predicate)

    fun testKeyDocDescribesCodegenVersion() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        val doc = KonvoyTomlDocumentationProvider()
            .generateDoc(key(file) { it.text == "version" }, null)
        assertNotNull(doc)
        assertTrue("doc was: $doc", doc!!.contains("Fabrikt"))
    }

    fun testKeyDocDescribesExtraSpecDirsAsOptional() {
        val file = myFixture.configureByText(
            "konvoy.toml",
            "$sample\nextra_spec_dirs = [\"specs\"]",
        )
        val doc = KonvoyTomlDocumentationProvider()
            .generateDoc(key(file) { it.text == "extra_spec_dirs" }, null)
        assertNotNull(doc)
        assertFalse("extra_spec_dirs is optional: $doc", doc!!.contains("Required"))
    }

    fun testHeaderDocDescribesCodegenSection() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        val doc = KonvoyTomlDocumentationProvider()
            .generateDoc(key(file) { it.text.contains("codegen") }, null)
        assertNotNull(doc)
        assertTrue("doc was: $doc", doc!!.contains("OpenAPI"))
    }
}
