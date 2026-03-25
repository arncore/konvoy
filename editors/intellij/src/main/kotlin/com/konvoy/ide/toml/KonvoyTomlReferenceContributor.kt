package com.konvoy.ide.toml

import com.intellij.openapi.util.TextRange
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.patterns.PlatformPatterns
import com.intellij.psi.*
import com.intellij.util.ProcessingContext
import org.toml.lang.psi.TomlKeyValue
import org.toml.lang.psi.TomlLiteral
import java.io.File

/**
 * Adds clickable navigation references in `konvoy.toml`:
 * - `entrypoint = "src/main.kt"` → navigates to the file
 * - `path = "../mylib"` → navigates to the dependency directory
 */
class KonvoyTomlReferenceContributor : PsiReferenceContributor() {

    override fun registerReferenceProviders(registrar: PsiReferenceRegistrar) {
        registrar.registerReferenceProvider(
            PlatformPatterns.psiElement(TomlLiteral::class.java),
            KonvoyTomlPathReferenceProvider(),
        )
    }
}

private class KonvoyTomlPathReferenceProvider : PsiReferenceProvider() {

    override fun getReferencesByElement(element: PsiElement, context: ProcessingContext): Array<PsiReference> {
        if (element.containingFile?.name != "konvoy.toml") return PsiReference.EMPTY_ARRAY

        val literal = element as? TomlLiteral ?: return PsiReference.EMPTY_ARRAY
        val kv = literal.parent as? TomlKeyValue ?: return PsiReference.EMPTY_ARRAY
        val keyName = kv.key.text
        val tableName = KonvoyTomlPsiUtils.findContainingTableName(kv) ?: return PsiReference.EMPTY_ARRAY

        // entrypoint = "src/main.kt" in [package]
        if (tableName == "package" && keyName == "entrypoint") {
            return createFileReference(literal)
        }

        // path = "../mylib" in [dependencies.*] or [plugins.*]
        if ((tableName.startsWith("dependencies.") || tableName.startsWith("plugins.")) && keyName == "path") {
            return createDirectoryReference(literal)
        }

        return PsiReference.EMPTY_ARRAY
    }

    private fun createFileReference(literal: TomlLiteral): Array<PsiReference> {
        val path = stripQuotes(literal.text) ?: return PsiReference.EMPTY_ARRAY
        if (path.isEmpty()) return PsiReference.EMPTY_ARRAY
        val range = getValueTextRange(literal) ?: return PsiReference.EMPTY_ARRAY
        return arrayOf(KonvoyFileReference(literal, range, path, expectDirectory = false))
    }

    private fun createDirectoryReference(literal: TomlLiteral): Array<PsiReference> {
        val path = stripQuotes(literal.text) ?: return PsiReference.EMPTY_ARRAY
        if (path.isEmpty()) return PsiReference.EMPTY_ARRAY
        val range = getValueTextRange(literal) ?: return PsiReference.EMPTY_ARRAY
        return arrayOf(KonvoyFileReference(literal, range, path, expectDirectory = true))
    }

    private fun stripQuotes(text: String): String? {
        return when {
            text.startsWith("\"") && text.endsWith("\"") -> text.removeSurrounding("\"")
            text.startsWith("'") && text.endsWith("'") -> text.removeSurrounding("'")
            else -> null
        }
    }

    private fun getValueTextRange(literal: TomlLiteral): TextRange? {
        val text = literal.text
        if (text.length < 2) return null
        return TextRange(1, text.length - 1)
    }
}

private class KonvoyFileReference(
    element: PsiElement,
    range: TextRange,
    private val relativePath: String,
    private val expectDirectory: Boolean,
) : PsiReferenceBase<PsiElement>(element, range, true) {

    override fun resolve(): PsiElement? {
        val basePath = element.project.basePath ?: return null
        val targetPath = File(basePath, relativePath).canonicalPath
        val vFile = LocalFileSystem.getInstance().findFileByPath(targetPath) ?: return null

        if (expectDirectory && !vFile.isDirectory) return null
        if (!expectDirectory && vFile.isDirectory) return null

        return PsiManager.getInstance(element.project).findFile(vFile)
            ?: PsiManager.getInstance(element.project).findDirectory(vFile)
    }
}
