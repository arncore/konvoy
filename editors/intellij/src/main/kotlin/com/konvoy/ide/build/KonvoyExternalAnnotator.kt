package com.konvoy.ide.build

import com.intellij.lang.annotation.AnnotationHolder
import com.intellij.lang.annotation.ExternalAnnotator
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.util.TextRange
import com.intellij.psi.PsiFile
import com.konvoy.ide.sync.KonvoyProjectService

/**
 * Displays compiler diagnostics from the last background build as inline
 * error/warning markers in Kotlin editors. Reads from [KonvoyExternalAnnotatorService].
 */
class KonvoyExternalAnnotator : ExternalAnnotator<PsiFile, List<KonvoyDiagnosticsParser.ParsedDiagnostic>>() {

    override fun collectInformation(file: PsiFile, editor: Editor, hasErrors: Boolean): PsiFile? {
        val project = file.project
        if (!KonvoyProjectService.getInstance(project).isKonvoyProject) return null
        if (file.virtualFile?.extension != "kt") return null
        return file
    }

    override fun doAnnotate(file: PsiFile): List<KonvoyDiagnosticsParser.ParsedDiagnostic> {
        val vFile = file.virtualFile ?: return emptyList()
        return KonvoyExternalAnnotatorService.getInstance(file.project).getDiagnostics(vFile)
    }

    override fun apply(file: PsiFile, diagnostics: List<KonvoyDiagnosticsParser.ParsedDiagnostic>, holder: AnnotationHolder) {
        val document = file.viewProvider.document ?: return

        for (diag in diagnostics) {
            val lineIndex = (diag.line - 1).coerceIn(0, document.lineCount - 1)
            val lineStart = document.getLineStartOffset(lineIndex)
            val lineEnd = document.getLineEndOffset(lineIndex)

            val offset = if (diag.column != null) {
                (lineStart + diag.column - 1).coerceIn(lineStart, lineEnd)
            } else {
                lineStart
            }

            val severity = when (diag.severity) {
                KonvoyDiagnosticsParser.Severity.ERROR -> HighlightSeverity.ERROR
                KonvoyDiagnosticsParser.Severity.WARNING -> HighlightSeverity.WARNING
                KonvoyDiagnosticsParser.Severity.INFO -> HighlightSeverity.INFORMATION
            }

            val rangeStart = offset.coerceIn(lineStart, lineEnd)
            val rangeEnd = lineEnd.coerceAtLeast(rangeStart)

            holder.newAnnotation(severity, diag.message)
                .range(TextRange(rangeStart, rangeEnd))
                .create()
        }
    }
}
