package com.konvoy.ide.build

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.konvoy.ide.build.KonvoyDiagnosticsParser.ParsedDiagnostic
import java.io.File

/**
 * Runs `konvoy build --verbose` in the background and applies
 * compiler diagnostics as inline editor annotations.
 */
object KonvoyBackgroundBuilder {

    private val LOG = Logger.getInstance(KonvoyBackgroundBuilder::class.java)

    fun build(project: Project) {
        val basePath = project.basePath ?: return
        val status = KonvoyBuildStatusService.getInstance(project)

        com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
            status.setBuilding()
        }

        try {
            val process = ProcessBuilder("konvoy", "build", "--verbose")
                .directory(File(basePath))
                .redirectErrorStream(true)
                .start()

            val output = process.inputStream.bufferedReader().readText()
            val finished = process.waitFor(60, java.util.concurrent.TimeUnit.SECONDS)
            if (!finished) {
                process.destroyForcibly()
                LOG.warn("Background build timed out after 60s")
                com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
                    status.setResult(1, 0) // Show as error
                }
                return
            }

            LOG.info("Background build finished (exit=${process.exitValue()})")

            val diagnostics = KonvoyDiagnosticsParser.parseKonanc(output)

            com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
                if (project.isDisposed) return@invokeLater
                KonvoyDiagnosticsApplier.apply(project, basePath, diagnostics)

                val errors = diagnostics.count { it.severity == KonvoyDiagnosticsParser.Severity.ERROR }
                val warnings = diagnostics.count { it.severity == KonvoyDiagnosticsParser.Severity.WARNING }
                status.setResult(errors, warnings)
            }
        } catch (e: Exception) {
            LOG.warn("Background build failed", e)
            com.intellij.openapi.application.ApplicationManager.getApplication().invokeLater {
                status.clear()
            }
        }
    }
}

/**
 * Parses konanc compiler output into structured diagnostics.
 * Matches the same patterns as the VS Code extension's `parseKonancDiagnostics`.
 */
object KonvoyDiagnosticsParser {

    data class ParsedDiagnostic(
        val file: String,
        val line: Int,      // 1-based
        val column: Int?,   // 1-based, nullable
        val severity: Severity,
        val message: String,
    )

    enum class Severity { ERROR, WARNING, INFO }

    private val LOCATED_RE = Regex("^(.+\\.kt):(\\d+):(?:(\\d+):)?\\s*(error|warning|info):\\s*(.*)$")

    fun parseKonanc(output: String): List<ParsedDiagnostic> {
        return output.lines().mapNotNull { line ->
            val match = LOCATED_RE.matchEntire(line.trim()) ?: return@mapNotNull null
            ParsedDiagnostic(
                file = match.groupValues[1],
                line = match.groupValues[2].toIntOrNull() ?: return@mapNotNull null,
                column = match.groupValues[3].toIntOrNull(),
                severity = when (match.groupValues[4]) {
                    "error" -> Severity.ERROR
                    "warning" -> Severity.WARNING
                    else -> Severity.INFO
                },
                message = match.groupValues[5],
            )
        }
    }
}

/**
 * Applies parsed diagnostics as IntelliJ external annotations (Problems view + editor markers).
 */
object KonvoyDiagnosticsApplier {

    private val LOG = Logger.getInstance(KonvoyDiagnosticsApplier::class.java)

    fun apply(project: Project, basePath: String, diagnostics: List<ParsedDiagnostic>) {
        val service = KonvoyExternalAnnotatorService.getInstance(project)
        service.clearDiagnostics()

        for (diag in diagnostics) {
            val resolvedPath = if (File(diag.file).isAbsolute) diag.file else File(basePath, diag.file).canonicalPath
            val vFile = LocalFileSystem.getInstance().findFileByPath(resolvedPath) ?: continue
            service.addDiagnostic(vFile, diag)
        }

        // Trigger re-highlighting of open editors
        com.intellij.codeInsight.daemon.DaemonCodeAnalyzer.getInstance(project).restart()

        val errorCount = diagnostics.count { it.severity == KonvoyDiagnosticsParser.Severity.ERROR }
        val warnCount = diagnostics.count { it.severity == KonvoyDiagnosticsParser.Severity.WARNING }
        if (errorCount > 0 || warnCount > 0) {
            LOG.info("Build diagnostics: $errorCount error(s), $warnCount warning(s)")
        }
    }
}
