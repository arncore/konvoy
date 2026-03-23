package com.konvoy.ide.sync

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManager
import com.intellij.openapi.vfs.newvfs.BulkFileListener
import com.intellij.openapi.vfs.newvfs.events.VFileEvent
import com.intellij.util.Alarm
import com.intellij.util.Alarm.ThreadToUse

/**
 * Watches for changes to konvoy.toml and konvoy.lock and triggers re-sync.
 * Uses debouncing to avoid excessive syncs during rapid edits.
 */
class KonvoyFileListener : BulkFileListener {
    private val alarm = Alarm(ThreadToUse.POOLED_THREAD)
    private val debounceMs = 500

    override fun after(events: MutableList<out VFileEvent>) {
        val dominated = events.any { event ->
            val name = event.file?.name ?: event.path.substringAfterLast('/')
            name == "konvoy.toml" || name == "konvoy.lock"
        }
        if (!dominated) return

        alarm.cancelAllRequests()
        alarm.addRequest({
            for (project in ProjectManager.getInstance().openProjects) {
                if (project.isDisposed) continue
                val service = KonvoyProjectService.getInstance(project)
                if (service.isKonvoyProject) {
                    LOG.info("Konvoy config changed, re-syncing ${project.name}")
                    service.sync()
                }
            }
        }, debounceMs)
    }

    companion object {
        private val LOG = Logger.getInstance(KonvoyFileListener::class.java)
    }
}
