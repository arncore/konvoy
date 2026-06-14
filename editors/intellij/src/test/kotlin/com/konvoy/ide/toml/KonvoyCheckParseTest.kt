package com.konvoy.ide.toml

import junit.framework.TestCase

/**
 * Pure-logic tests for parsing `konvoy check --format json` output. No IntelliJ
 * platform needed — this only exercises [KonvoyCheck.parse].
 */
class KonvoyCheckParseTest : TestCase() {

    fun testEmptyArrayIsNoDiagnostics() {
        assertTrue(KonvoyCheck.parse("[]").isEmpty())
    }

    fun testSemanticDiagnosticCarriesKeyPathNotLine() {
        val json =
            """[{"severity":"error","message":"spec must point to an OpenAPI .yaml, .yml, or .json file","key_path":"codegen.openapi"}]"""
        val diags = KonvoyCheck.parse(json)
        assertEquals(1, diags.size)
        val d = diags[0]
        assertEquals("error", d.severity)
        assertEquals("codegen.openapi", d.keyPath)
        assertNull(d.line)
        assertNull(d.column)
        assertTrue(d.message.contains(".yaml"))
    }

    fun testSyntaxDiagnosticCarriesLineAndColumn() {
        val json = """[{"severity":"error","message":"TOML parse error","line":1,"column":9}]"""
        val diags = KonvoyCheck.parse(json)
        assertEquals(1, diags.size)
        assertEquals(1, diags[0].line)
        assertEquals(9, diags[0].column)
        assertNull(diags[0].keyPath)
    }

    fun testMultipleDiagnosticsPreserveOrder() {
        val json =
            """[{"severity":"error","message":"a","key_path":"package.name"},{"severity":"error","message":"b","line":2,"column":3}]"""
        val diags = KonvoyCheck.parse(json)
        assertEquals(2, diags.size)
        assertEquals("package.name", diags[0].keyPath)
        assertEquals(2, diags[1].line)
    }

    fun testMalformedJsonIsNoDiagnostics() {
        assertTrue(KonvoyCheck.parse("not json").isEmpty())
        assertTrue(KonvoyCheck.parse("").isEmpty())
        assertTrue(KonvoyCheck.parse("{}").isEmpty())
    }

    fun testEntryWithoutMessageIsSkipped() {
        assertTrue(KonvoyCheck.parse("""[{"severity":"error","key_path":"package.name"}]""").isEmpty())
    }
}
