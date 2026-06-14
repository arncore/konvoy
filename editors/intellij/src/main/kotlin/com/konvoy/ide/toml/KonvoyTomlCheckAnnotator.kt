package com.konvoy.ide.toml

import com.intellij.lang.annotation.AnnotationHolder
import com.intellij.lang.annotation.ExternalAnnotator
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.openapi.editor.Document
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.util.TextRange
import com.intellij.psi.PsiFile
import com.konvoy.ide.sync.KonvoyProjectService
import java.io.File

/**
 * Validates `konvoy.toml` by running `konvoy check --format json` off the UI thread
 * and rendering the backend's diagnostics inline.
 *
 * The plugin is a thin client — it never re-implements konvoy's validation rules. It
 * only locates each reported diagnostic (by its dotted key path, or by line/column
 * for TOML syntax errors) and shows konvoy's message. Diagnostics are reported for
 * the saved `konvoy.toml` on disk, which is what `konvoy build`/`generate` will read.
 */
class KonvoyTomlCheckAnnotator :
    ExternalAnnotator<KonvoyTomlCheckAnnotator.Request, List<KonvoyCheckDiagnostic>>() {

    /** Carries the project directory from the UI thread into [doAnnotate]. */
    data class Request(val projectDir: File)

    override fun collectInformation(file: PsiFile, editor: Editor, hasErrors: Boolean): Request? {
        if (file.name != "konvoy.toml") return null
        if (!KonvoyProjectService.getInstance(file.project).isKonvoyProject) return null
        val dir = file.virtualFile?.parent?.path ?: return null
        return Request(File(dir))
    }

    override fun doAnnotate(request: Request): List<KonvoyCheckDiagnostic> =
        KonvoyCheck.run(request.projectDir)

    override fun apply(
        file: PsiFile,
        diagnostics: List<KonvoyCheckDiagnostic>,
        holder: AnnotationHolder,
    ) {
        val document = file.viewProvider.document ?: return
        for (diag in diagnostics) {
            val range = rangeFor(file, document, diag) ?: continue
            holder.newAnnotation(severityOf(diag.severity), diag.message)
                .range(range)
                .create()
        }
    }

    private fun severityOf(severity: String): HighlightSeverity = when (severity) {
        "warning" -> HighlightSeverity.WARNING
        else -> HighlightSeverity.ERROR
    }

    /** Locate a diagnostic: prefer its TOML key path, fall back to line/column. */
    private fun rangeFor(file: PsiFile, document: Document, diag: KonvoyCheckDiagnostic): TextRange? {
        diag.keyPath?.let { keyPath ->
            KonvoyTomlPsiUtils.findElementByKeyPath(file, keyPath)?.textRange?.let { return it }
        }
        if (document.lineCount == 0) return null
        val line = diag.line ?: return TextRange(document.getLineStartOffset(0), document.getLineEndOffset(0))
        val lineIndex = (line - 1).coerceIn(0, document.lineCount - 1)
        val lineStart = document.getLineStartOffset(lineIndex)
        val lineEnd = document.getLineEndOffset(lineIndex)
        val start = diag.column?.let { (lineStart + it - 1).coerceIn(lineStart, lineEnd) } ?: lineStart
        return TextRange(start, lineEnd.coerceAtLeast(start))
    }
}
