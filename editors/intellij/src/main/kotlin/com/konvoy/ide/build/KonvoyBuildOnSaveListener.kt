package com.konvoy.ide.build

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.editor.Document
import com.intellij.openapi.fileEditor.FileDocumentManagerListener
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.project.Project
import kotlinx.coroutines.*

/**
 * Triggers a background `konvoy build --verbose` when a `.kt` file is saved
 * in a Konvoy project, then applies compiler diagnostics as inline error markers.
 *
 * Debounced with 1s delay to avoid spamming builds on rapid saves.
 * Can be disabled via Settings > Build > Konvoy > Build on save.
 *
 * Registered per-project in [com.konvoy.ide.sync.KonvoyStartupActivity],
 * auto-disposed when the project closes.
 */
class KonvoyBuildOnSaveListener(private val project: Project) : FileDocumentManagerListener {

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var buildJob: Job? = null
    private val debounceMs = 1000L

    override fun beforeDocumentSaving(document: Document) {
        if (project.isDisposed) return

        val vFile = com.intellij.openapi.fileEditor.FileDocumentManager.getInstance().getFile(document)
            ?: return

        if (!isKotlinSource(vFile)) return

        val settings = KonvoyBuildSettings.getInstance(project)
        if (!settings.state.buildOnSave) return

        val basePath = project.basePath ?: return
        if (!vFile.path.startsWith(basePath)) return

        buildJob?.cancel()
        if (project.isDisposed) return
        buildJob = scope.launch {
            delay(debounceMs)
            if (project.isDisposed) return@launch
            LOG.info("Build on save triggered for ${vFile.name}")
            KonvoyBackgroundBuilder.build(project)
        }
    }

    private fun isKotlinSource(file: VirtualFile): Boolean {
        return file.extension == "kt" && !file.path.contains("/.konvoy/")
    }

    companion object {
        private val LOG = Logger.getInstance(KonvoyBuildOnSaveListener::class.java)
    }
}
