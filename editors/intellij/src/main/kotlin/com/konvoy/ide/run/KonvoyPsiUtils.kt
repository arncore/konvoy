package com.konvoy.ide.run

import com.intellij.openapi.project.Project
import com.intellij.psi.PsiElement
import com.konvoy.ide.config.PackageKind
import com.konvoy.ide.sync.KonvoyProjectService
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

        val filePath = element.containingFile?.virtualFile?.path ?: return null
        val basePath = project.basePath ?: return null
        if (!filePath.startsWith("$basePath/src/test/")) return null

        val hasTestAnnotation = function.annotationEntries.any { annotation ->
            annotation.shortName?.asString() == "Test"
        }
        if (!hasTestAnnotation) return null

        return function
    }
}
