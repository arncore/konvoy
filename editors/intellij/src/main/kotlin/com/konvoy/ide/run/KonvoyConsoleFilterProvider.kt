package com.konvoy.ide.run

import com.intellij.execution.filters.ConsoleFilterProvider
import com.intellij.execution.filters.Filter
import com.intellij.execution.filters.OpenFileHyperlinkInfo
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import java.io.File
import java.util.regex.Pattern

/**
 * Parses konanc and detekt output to make file:line:col references clickable
 * in the run console. Matches the same patterns as the VS Code extension's
 * `parseKonancDiagnostics` and `parseDetektDiagnostics`.
 */
class KonvoyConsoleFilterProvider : ConsoleFilterProvider {
    override fun getDefaultFilters(project: Project): Array<Filter> {
        return arrayOf(KonvoyConsoleFilter(project))
    }
}

private class KonvoyConsoleFilter(private val project: Project) : Filter {

    // konanc: file.kt:3:5: error: message
    // konanc: file.kt:3: warning: message (no column)
    private val KONANC_RE = Pattern.compile("^(.+\\.kt):(\\d+):(?:(\\d+):)?\\s*(?:error|warning|info):\\s*(.*)$")

    // detekt: file.kt:3:5: message text [RuleName]
    private val DETEKT_RE = Pattern.compile("^(.+\\.kt):(\\d+):(\\d+):\\s*.+\\[\\w+]$")

    override fun applyFilter(line: String, entireLength: Int): Filter.Result? {
        val trimmed = line.trim()
        val lineStart = entireLength - line.length

        // Try konanc pattern
        val konancMatch = KONANC_RE.matcher(trimmed)
        if (konancMatch.matches()) {
            return createResult(
                konancMatch.group(1),
                konancMatch.group(2).toIntOrNull() ?: return null,
                konancMatch.group(3)?.toIntOrNull(),
                lineStart,
                line,
            )
        }

        // Try detekt pattern
        val detektMatch = DETEKT_RE.matcher(trimmed)
        if (detektMatch.matches()) {
            return createResult(
                detektMatch.group(1),
                detektMatch.group(2).toIntOrNull() ?: return null,
                detektMatch.group(3)?.toIntOrNull(),
                lineStart,
                line,
            )
        }

        return null
    }

    private fun createResult(
        filePath: String,
        lineNum: Int,
        colNum: Int?,
        lineStart: Int,
        line: String,
    ): Filter.Result? {
        val basePath = project.basePath ?: return null
        val resolvedPath = if (File(filePath).isAbsolute) filePath else File(basePath, filePath).canonicalPath
        val vFile = LocalFileSystem.getInstance().findFileByPath(resolvedPath) ?: return null

        // IntelliJ uses 0-based line/col, konanc/detekt use 1-based
        val zeroLine = (lineNum - 1).coerceAtLeast(0)
        val zeroCol = ((colNum ?: 1) - 1).coerceAtLeast(0)

        val linkInfo = OpenFileHyperlinkInfo(project, vFile, zeroLine, zeroCol)

        // Highlight just the file:line:col portion
        val fileRef = if (colNum != null) "$filePath:$lineNum:$colNum" else "$filePath:$lineNum"
        val highlightStart = lineStart + line.indexOf(fileRef).coerceAtLeast(0)
        val highlightEnd = highlightStart + fileRef.length

        return Filter.Result(highlightStart, highlightEnd, linkInfo)
    }
}
