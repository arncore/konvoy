package com.konvoy.ide.run

import com.intellij.execution.configurations.ParametersList
import junit.framework.TestCase

class KonvoyCommandLineArgsTest : TestCase() {

    fun testTestFilterExtraArgsKeepsSimpleFilterReadable() {
        assertFormatsAndParses(
            "MainTest.greetingIncludesName",
            "--filter MainTest.greetingIncludesName",
        )
    }

    fun testTestFilterExtraArgsKeepsWildcardFilterReadable() {
        assertFormatsAndParses(
            "MathTest.*",
            "--filter MathTest.*",
        )
    }

    fun testTestFilterExtraArgsKeepsSingleQuoteReadable() {
        assertFormatsAndParses(
            "MainTest.can't fail",
            "--filter \"MainTest.can't fail\"",
        )
    }

    fun testTestFilterExtraArgsQuotesFilterWithSpaces() {
        assertFormatsAndParses(
            "MainTest.greeting includes spaces",
            "--filter \"MainTest.greeting includes spaces\"",
        )
    }

    fun testTestFilterExtraArgsPreservesLeadingAndTrailingSpaces() {
        assertFormatsAndParses(
            "MainTest. leading and trailing ",
            "--filter \"MainTest. leading and trailing \"",
        )
    }

    fun testTestFilterExtraArgsQuotesTabs() {
        assertFormatsAndParses(
            "MainTest.name\twith tab",
            "--filter \"MainTest.name\twith tab\"",
        )
    }

    fun testTestFilterExtraArgsEscapesEmbeddedDoubleQuotes() {
        assertFormatsAndParses(
            "MainTest.says \"hi\"",
            "--filter \"MainTest.says \\\"hi\\\"\"",
        )
    }

    fun testTestFilterExtraArgsPreservesBackslashesWithoutQuotingWhenPossible() {
        assertFormatsAndParses(
            "MainTest.path\\segment",
            "--filter MainTest.path\\segment",
        )
    }

    fun testTestFilterExtraArgsPreservesBackslashesInsideQuotes() {
        assertFormatsAndParses(
            "MainTest.path C:\\tmp",
            "--filter \"MainTest.path C:\\tmp\"",
        )
    }

    fun testTestFilterExtraArgsQuotesWildcardFilterWithSpaces() {
        assertFormatsAndParses(
            "MainTest.* includes *",
            "--filter \"MainTest.* includes *\"",
        )
    }

    fun testTestFilterExtraArgsRejectsEmptyFilter() {
        try {
            KonvoyCommandLineArgs.testFilterExtraArgs("")
            fail("expected empty filters to be rejected")
        } catch (_: IllegalArgumentException) {
            // Expected.
        }
    }

    private fun assertFormatsAndParses(filter: String, expectedExtraArgs: String) {
        val extraArgs = KonvoyCommandLineArgs.testFilterExtraArgs(filter)

        assertEquals(expectedExtraArgs, extraArgs)
        assertEquals(
            listOf("--filter", filter),
            ParametersList.parse(extraArgs).toList(),
        )
    }
}
