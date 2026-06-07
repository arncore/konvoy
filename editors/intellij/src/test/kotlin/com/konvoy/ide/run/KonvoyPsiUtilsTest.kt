package com.konvoy.ide.run

import com.intellij.testFramework.fixtures.BasePlatformTestCase
import org.jetbrains.kotlin.psi.KtClassOrObject
import org.jetbrains.kotlin.psi.KtDeclaration
import org.jetbrains.kotlin.psi.KtFile
import org.jetbrains.kotlin.psi.KtNamedFunction

class KonvoyPsiUtilsTest : BasePlatformTestCase() {

    fun testTestSuiteFilterQualifiesClass() {
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

        val testClass = file.findClassOrObject("MainTest")

        assertEquals("MainTest.*", KonvoyPsiUtils.testSuiteFilter(testClass))
    }

    fun testTestSuiteFilterQualifiesNestedClass() {
        val file = myFixture.configureByText(
            "NestedTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                class Nested {
                    @Test
                    fun nestedGreetingIncludesName() {}
                }
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("Nested")

        assertEquals("MainTest.Nested.*", KonvoyPsiUtils.testSuiteFilter(testClass))
    }

    fun testTestSuiteFilterQualifiesDeeplyNestedClass() {
        val file = myFixture.configureByText(
            "DeepNestedTest.kt",
            """
            import kotlin.test.Test

            class Outer {
                class Middle {
                    class Inner {
                        @Test
                        fun deepTest() {}
                    }
                }
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("Inner")

        assertEquals("Outer.Middle.Inner.*", KonvoyPsiUtils.testSuiteFilter(testClass))
    }

    fun testTestSuiteFilterQualifiesTopLevelObject() {
        val file = myFixture.configureByText(
            "ObjectTest.kt",
            """
            import kotlin.test.Test

            object ObjectTest {
                @Test
                fun objectRoot() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testObject = file.findClassOrObject("ObjectTest")

        assertEquals("ObjectTest.*", KonvoyPsiUtils.testSuiteFilter(testObject))
    }

    fun testTestSuiteFilterKeepsBacktickClassNameWithSpaces() {
        val file = myFixture.configureByText(
            "BacktickClassTest.kt",
            """
            import kotlin.test.Test

            class `Main Test` {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("Main Test")

        assertEquals("Main Test.*", KonvoyPsiUtils.testSuiteFilter(testClass))
    }

    fun testFindTestSuiteDetectsTestClassNameIdentifierInTestSource() {
        val file = addProjectFile(
            "src/test/MainTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("MainTest")

        assertTrue(
            "expected ${testClass.containingFile.virtualFile.path} to be in src/test with basePath=${project.basePath}",
            KonvoyPsiUtils.isInTestSource(testClass.nameIdentifier!!, project),
        )
        assertSame(testClass, KonvoyPsiUtils.findTestSuite(testClass.nameIdentifier!!, project))
    }

    fun testFindTestSuiteDetectsTestClassElementInTestSource() {
        val file = addProjectFile(
            "src/test/MainTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("MainTest")

        assertTrue(
            "expected ${testClass.containingFile.virtualFile.path} to be in src/test with basePath=${project.basePath}",
            KonvoyPsiUtils.isInTestSource(testClass, project),
        )
        assertSame(testClass, KonvoyPsiUtils.findTestSuite(testClass, project))
    }

    fun testFindTestSuiteDetectsNestedTestClassNameIdentifierInTestSource() {
        val file = addProjectFile(
            "src/test/NestedTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                class Nested {
                    @Test
                    fun nestedGreetingIncludesName() {}
                }
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("Nested")

        assertSame(testClass, KonvoyPsiUtils.findTestSuite(testClass.nameIdentifier!!, project))
    }

    fun testFindTestSuiteDetectsTestObjectNameIdentifierInTestSource() {
        val file = addProjectFile(
            "src/test/ObjectTest.kt",
            """
            import kotlin.test.Test

            object ObjectTest {
                @Test
                fun objectRoot() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testObject = file.findClassOrObject("ObjectTest")

        assertSame(testObject, KonvoyPsiUtils.findTestSuite(testObject.nameIdentifier!!, project))
    }

    fun testFindTestSuiteIgnoresClassOutsideTestSource() {
        val file = addProjectFile(
            "src/main/MainTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("MainTest")

        assertNull(KonvoyPsiUtils.findTestSuite(testClass.nameIdentifier!!, project))
    }

    fun testFindTestSuiteIgnoresClassWithoutTestFunctions() {
        val file = addProjectFile(
            "src/test/Helper.kt",
            """
            class Helper {
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val testClass = file.findClassOrObject("Helper")

        assertNull(KonvoyPsiUtils.findTestSuite(testClass.nameIdentifier!!, project))
    }

    fun testFindTestSuiteIgnoresFunctionNameIdentifier() {
        val file = addProjectFile(
            "src/test/MainTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun greetingIncludesName() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("greetingIncludesName")

        assertNull(KonvoyPsiUtils.findTestSuite(function.nameIdentifier!!, project))
    }

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

    fun testTestFilterQualifiesNestedClassMethod() {
        val file = myFixture.configureByText(
            "NestedTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                class Nested {
                    @Test
                    fun nestedGreetingIncludesName() {}
                }
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("nestedGreetingIncludesName")

        assertEquals(
            "MainTest.Nested.nestedGreetingIncludesName",
            KonvoyPsiUtils.testFilter(function),
        )
    }

    fun testTestFilterQualifiesDeeplyNestedClassMethod() {
        val file = myFixture.configureByText(
            "DeepNestedTest.kt",
            """
            import kotlin.test.Test

            class Outer {
                class Middle {
                    class Inner {
                        @Test
                        fun deepTest() {}
                    }
                }
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("deepTest")

        assertEquals("Outer.Middle.Inner.deepTest", KonvoyPsiUtils.testFilter(function))
    }

    fun testTestFilterQualifiesNestedObjectMethod() {
        val file = myFixture.configureByText(
            "NestedObjectTest.kt",
            """
            import kotlin.test.Test

            class Outer {
                object NestedObject {
                    @Test
                    fun objectTest() {}
                }
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("objectTest")

        assertEquals("Outer.NestedObject.objectTest", KonvoyPsiUtils.testFilter(function))
    }

    fun testTestFilterQualifiesTopLevelObjectMethod() {
        val file = myFixture.configureByText(
            "ObjectTest.kt",
            """
            import kotlin.test.Test

            object ObjectTest {
                @Test
                fun objectRoot() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("objectRoot")

        assertEquals("ObjectTest.objectRoot", KonvoyPsiUtils.testFilter(function))
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

    fun testTestFilterKeepsBacktickFunctionNameWithSpaces() {
        val file = myFixture.configureByText(
            "BacktickTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun `greeting includes spaces`() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("greeting includes spaces")

        assertEquals("MainTest.greeting includes spaces", KonvoyPsiUtils.testFilter(function))
    }

    fun testTestFilterKeepsBacktickFunctionNameWithQuotes() {
        val file = myFixture.configureByText(
            "QuotedBacktickTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun `says "hi"`() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction("says \"hi\"")

        assertEquals("MainTest.says \"hi\"", KonvoyPsiUtils.testFilter(function))
    }

    fun testTestFilterKeepsBacktickFunctionNameWithLeadingAndTrailingSpaces() {
        val file = myFixture.configureByText(
            "LeadingTrailingBacktickTest.kt",
            """
            import kotlin.test.Test

            class MainTest {
                @Test
                fun ` leading and trailing `() {}
            }
            """.trimIndent(),
        ) as KtFile

        val function = file.findFunction(" leading and trailing ")

        assertEquals("MainTest. leading and trailing ", KonvoyPsiUtils.testFilter(function))
    }

    private fun addProjectFile(path: String, text: String): KtFile =
        myFixture.addFileToProject(path, text.trimIndent()) as KtFile

    private fun KtFile.findFunction(name: String): KtNamedFunction =
        declarations.flatMap { declaration -> declaration.functions() }
            .single { it.name == name }

    private fun KtFile.findClassOrObject(name: String): KtClassOrObject =
        declarations.flatMap { declaration -> declaration.classOrObjects() }
            .single { it.name == name }

    private fun KtDeclaration.functions(): List<KtNamedFunction> =
        when (this) {
            is KtNamedFunction -> listOf(this)
            is KtClassOrObject -> declarations.flatMap { it.functions() }
            else -> emptyList()
        }

    private fun KtDeclaration.classOrObjects(): List<KtClassOrObject> =
        when (this) {
            is KtClassOrObject -> listOf(this) + declarations.flatMap { it.classOrObjects() }
            else -> emptyList()
        }
}
