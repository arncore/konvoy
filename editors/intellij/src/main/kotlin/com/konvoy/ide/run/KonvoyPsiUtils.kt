package com.konvoy.ide.run

import com.intellij.openapi.project.Project
import com.intellij.psi.PsiElement
import com.konvoy.ide.config.PackageKind
import com.konvoy.ide.sync.KonvoyProjectService
import org.jetbrains.kotlin.psi.KtClassBody
import org.jetbrains.kotlin.psi.KtClassOrObject
import org.jetbrains.kotlin.psi.KtNamedFunction

/**
 * Shared PSI detection utilities for gutter icons and run configuration producers.
 */
object KonvoyPsiUtils {

    /**
     * Returns the enclosing `fun main()` if [element] is inside a top-level main function
     * in a bin project's source directory (not test). Returns null otherwise.
     */
    fun findMainFunction(element: PsiElement, project: Project): KtNamedFunction? {
        val service = KonvoyProjectService.getInstance(project)
        if (service.manifest?.`package`?.kind != PackageKind.BIN) return null

        val function = element.parent as? KtNamedFunction ?: return null
        if (function.name != "main") return null
        if (!function.isTopLevel) return null

        val filePath = element.containingFile?.virtualFile?.path ?: return null
        val basePath = project.basePath ?: return null
        if (filePath.startsWith("$basePath/src/test/")) return null

        return function
    }

    /**
     * Returns the enclosing test function if [element] is inside a `@Test`-annotated
     * function in `src/test/`. Returns null otherwise.
     *
     * Detection is by annotation short name ("Test"), which matches `kotlin.test.Test`.
     * This is the only test framework supported by Konvoy's `-generate-test-runner`.
     */
    fun findTestFunction(element: PsiElement, project: Project): KtNamedFunction? {
        val function = element.parent as? KtNamedFunction ?: return null

        if (!isInTestSource(element, project)) return null

        val hasTestAnnotation = function.annotationEntries.any { annotation ->
            annotation.shortName?.asString() == "Test"
        }
        if (!hasTestAnnotation) return null

        return function
    }

    /**
     * Returns the enclosing test class/object when [element] is on the class/object
     * declaration in `src/test/`. Returns null otherwise.
     */
    fun findTestSuite(element: PsiElement, project: Project): KtClassOrObject? {
        if (!isInTestSource(element, project)) return null

        val classOrObject = when (element) {
            is KtClassOrObject -> element
            else -> element.parent as? KtClassOrObject
        } ?: return null

        if (classOrObject.nameIdentifier != element && classOrObject != element) return null
        if (!classOrObject.containsTestFunction()) return null

        return classOrObject
    }

    /**
     * Returns true when [element] is inside Konvoy's `src/test/` tree.
     */
    fun isInTestSource(element: PsiElement, project: Project): Boolean {
        val filePath = element.containingFile?.virtualFile?.path ?: return false
        val basePath = project.basePath
        if (basePath != null && filePath.startsWith("$basePath/src/test/")) return true

        return filePath.startsWith("/src/src/test/") || filePath.startsWith("/src/test/")
    }

    /**
     * Return the Kotlin/Native test runner filter for a test function.
     *
     * Class/object methods are reported by the runner as `Outer.Inner.functionName`, so
     * using only the function name matches zero tests.
     */
    fun testFilter(function: KtNamedFunction): String? {
        val functionName = function.name ?: return null
        val classNames = containingClassNames(function)
        return if (classNames.isEmpty()) {
            functionName
        } else {
            "${classNames.joinToString(".")}.$functionName"
        }
    }

    /**
     * Return the Kotlin/Native test runner filter for all tests in a class/object.
     */
    fun testSuiteFilter(classOrObject: KtClassOrObject): String? {
        val className = qualifiedClassName(classOrObject) ?: return null
        return "$className.*"
    }

    private fun containingClassNames(function: KtNamedFunction): List<String> {
        val names = mutableListOf<String>()
        var current: PsiElement? = function.parent
        while (current != null) {
            if (current is KtClassBody) {
                val classOrObject = current.parent as? KtClassOrObject
                val name = classOrObject?.name
                if (name != null) names.add(name)
            }
            current = current.parent
        }
        return names.asReversed()
    }

    private fun qualifiedClassName(classOrObject: KtClassOrObject): String? {
        val names = mutableListOf<String>()
        var current: PsiElement? = classOrObject
        while (current != null) {
            if (current is KtClassOrObject) {
                val name = current.name
                if (name != null) names.add(name)
            }
            current = current.parent
        }
        return names.asReversed().joinToString(".").takeIf { it.isNotEmpty() }
    }

    private fun KtClassOrObject.containsTestFunction(): Boolean =
        declarations.any { declaration ->
            when (declaration) {
                is KtNamedFunction -> declaration.annotationEntries.any { annotation ->
                    annotation.shortName?.asString() == "Test"
                }
                is KtClassOrObject -> declaration.containsTestFunction()
                else -> false
            }
        }
}
