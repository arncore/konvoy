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
     * Returns true when [element] is inside Konvoy's `src/test/` tree.
     */
    fun isInTestSource(element: PsiElement, project: Project): Boolean {
        val filePath = element.containingFile?.virtualFile?.path ?: return false
        val basePath = project.basePath ?: return false
        return filePath.startsWith("$basePath/src/test/")
    }

    /**
     * Return the Kotlin/Native test runner filter for a test function.
     *
     * Class methods are reported by the runner as `ClassName.functionName`, so
     * using only the function name matches zero tests.
     */
    fun testFilter(function: KtNamedFunction): String? {
        val functionName = function.name ?: return null
        val className = containingClass(function)?.name
        return if (className != null) "$className.$functionName" else functionName
    }

    private fun containingClass(function: KtNamedFunction): KtClassOrObject? {
        val classBody = function.parent as? KtClassBody ?: return null
        return classBody.parent as? KtClassOrObject
    }
}
