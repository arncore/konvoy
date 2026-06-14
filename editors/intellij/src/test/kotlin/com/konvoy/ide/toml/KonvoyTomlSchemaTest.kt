package com.konvoy.ide.toml

import junit.framework.TestCase

/**
 * Pure-logic tests for the konvoy.toml completion schema (no IntelliJ platform).
 */
class KonvoyTomlSchemaTest : TestCase() {

    fun testCodegenIsATopLevelSection() {
        assertTrue("codegen" in KonvoyTomlSchema.SECTIONS)
    }

    fun testCodegenOpenApiKeysMatchTheShippedConfig() {
        val keys = KonvoyTomlSchema.keysForSection("codegen.openapi")
        assertNotNull(keys)
        // Shipped config — note `extra_spec_dirs`, NOT the old `spec_dirs`.
        assertEquals(
            setOf("version", "spec", "base_package", "extra_spec_dirs"),
            keys!!.keys,
        )
        assertTrue(keys.getValue("version").required)
        assertTrue(keys.getValue("spec").required)
        assertTrue(keys.getValue("base_package").required)
        assertFalse(keys.getValue("extra_spec_dirs").required)
    }

    fun testDoesNotReintroduceStaleSpecDirsKey() {
        val keys = KonvoyTomlSchema.keysForSection("codegen.openapi")!!
        assertFalse("spec_dirs" in keys)
    }

    fun testUnknownSectionHasNoKeys() {
        assertNull(KonvoyTomlSchema.keysForSection("codegen.grpc"))
    }
}
