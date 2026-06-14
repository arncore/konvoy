package com.konvoy.ide.toml

import com.intellij.testFramework.fixtures.BasePlatformTestCase

/**
 * Verifies that a `konvoy check` diagnostic's dotted key path resolves to the right
 * `konvoy.toml` PSI element — the placement logic the thin annotator relies on.
 */
class KonvoyTomlKeyPathTest : BasePlatformTestCase() {

    private val sample = """
        [package]
        name = "demo"

        [toolchain]
        kotlin = "2.2.0"

        [dependencies]
        foo = { maven = "g:a", version = "1.0.0" }

        [codegen.openapi]
        version = "20.0.0"
        spec = "specs/api.yaml"
        base_package = "com.example.api"
    """.trimIndent()

    fun testKeyPathResolvesToKeyInTable() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        val element = KonvoyTomlPsiUtils.findElementByKeyPath(file, "package.name")
        assertNotNull(element)
        assertEquals("name", element!!.text)
    }

    fun testKeyPathResolvesToTableHeaderWhenPathIsATable() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        val element = KonvoyTomlPsiUtils.findElementByKeyPath(file, "codegen.openapi")
        assertNotNull(element)
        assertTrue(element!!.text.contains("codegen.openapi"))
    }

    fun testKeyPathResolvesDependencyKeyUnderDependenciesTable() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        val element = KonvoyTomlPsiUtils.findElementByKeyPath(file, "dependencies.foo")
        assertNotNull(element)
        assertEquals("foo", element!!.text)
    }

    fun testUnknownKeyPathResolvesToNull() {
        val file = myFixture.configureByText("konvoy.toml", sample)
        assertNull(KonvoyTomlPsiUtils.findElementByKeyPath(file, "nonexistent.thing"))
    }
}
