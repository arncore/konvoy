package com.konvoy.ide.build

import com.intellij.openapi.components.Service
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList

/**
 * Project-level service that stores compiler diagnostics from the last
 * background build. The [KonvoyExternalAnnotator] reads from this service
 * to display inline error markers in editors.
 */
@Service(Service.Level.PROJECT)
class KonvoyExternalAnnotatorService(private val project: Project) {

    private val diagnosticsByFile = ConcurrentHashMap<String, CopyOnWriteArrayList<KonvoyDiagnosticsParser.ParsedDiagnostic>>()

    fun addDiagnostic(file: VirtualFile, diagnostic: KonvoyDiagnosticsParser.ParsedDiagnostic) {
        diagnosticsByFile.getOrPut(file.path) { CopyOnWriteArrayList() }.add(diagnostic)
    }

    fun getDiagnostics(file: VirtualFile): List<KonvoyDiagnosticsParser.ParsedDiagnostic> {
        return diagnosticsByFile[file.path] ?: emptyList()
    }

    fun clearDiagnostics() {
        diagnosticsByFile.clear()
    }

    companion object {
        fun getInstance(project: Project): KonvoyExternalAnnotatorService =
            project.getService(KonvoyExternalAnnotatorService::class.java)
    }
}
