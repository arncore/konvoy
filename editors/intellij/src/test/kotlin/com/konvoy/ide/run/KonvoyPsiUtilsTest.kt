package com.konvoy.ide.run

import com.intellij.testFramework.fixtures.BasePlatformTestCase
import org.jetbrains.kotlin.psi.KtClassOrObject
import org.jetbrains.kotlin.psi.KtFile
import org.jetbrains.kotlin.psi.KtNamedFunction

class KonvoyPsiUtilsTest : BasePlatformTestCase() {

    fun testTestFilterQualifiesClassMethod() {
        val file = myFixture.configureByText(
            "MainTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("greetingIncludesName")

        assertEquals("MainTest.greetingIncludesName", KonvoyPsiUtils.testFilter(function))
    }

    fun testTestFilterKeepsTopLevelFunctionName() {
        val file = myFixture.configureByText(
            "TopLevelTest.kt",
            """
            import kotlin.test.Test

            @Test
            fun topLevelTest() {}
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("topLevelTest")

        assertEquals("topLevelTest", KonvoyPsiUtils.testFilter(function))
    }

    private fun KtFile.findFunction(name: String): KtNamedFunction =
        declarations
            .flatMap { declaration ->
                when (declaration) {
                    is KtNamedFunction -> listOf(declaration)
                    is KtClassOrObject -> declaration.declarations.filterIsInstance<KtNamedFunction>()
                    else -> emptyList()
                }
            }
            .single { it.name == name }
}
