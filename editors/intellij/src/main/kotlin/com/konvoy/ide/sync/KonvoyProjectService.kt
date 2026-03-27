package com.konvoy.ide.sync

import com.intellij.notification.NotificationAction
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.components.Service
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFile
import com.konvoy.ide.config.*

/**
 * Project-level service that holds the parsed Konvoy project model.
 * The single source of truth for the current konvoy.toml + konvoy.lock state.
 */
@Service(Service.Level.PROJECT)
class KonvoyProjectService(private val project: Project) {

    var manifest: KonvoyManifest? = null
        private set
    var lockfile: KonvoyLockfile? = null
        private set

    /** Whether this project has a konvoy.toml at its root. */
    val isKonvoyProject: Boolean
        get() = findManifestFile() != null

    fun findManifestFile(): VirtualFile? {
        val basePath = project.basePath ?: return null
        return LocalFileSystem.getInstance().findFileByPath("$basePath/konvoy.toml")
    }

    fun findLockfile(): VirtualFile? {
        val basePath = project.basePath ?: return null
        return LocalFileSystem.getInstance().findFileByPath("$basePath/konvoy.lock")
    }

    /**
     * Re-read konvoy.toml and konvoy.lock, update the parsed models,
     * and trigger a workspace model sync.
     *
     * @return true if sync succeeded, false if it failed
     */
    fun sync(): Boolean {
        val manifestFile = findManifestFile()
        if (manifestFile == null) {
            LOG.info("No konvoy.toml found in ${project.basePath}, skipping sync")
            manifest = null
            lockfile = null
            return false
        }

        try {
            manifest = ReadAction.compute<KonvoyManifest?, Throwable> {
                KonvoyTomlParser.parseManifest(project, manifestFile)
            }
        } catch (e: Throwable) {
            LOG.warn("Exception parsing konvoy.toml", e)
            notifySyncFailed("Failed to parse konvoy.toml: ${e.message}")
            return false
        }

        if (manifest == null) {
            LOG.warn("Failed to parse konvoy.toml")
            notifySyncFailed("Failed to parse konvoy.toml. Check the file for syntax errors.")
            return false
        }

        val lockfileFile = findLockfile()
        lockfile = lockfileFile?.let {
            try {
                ReadAction.compute<KonvoyLockfile?, Throwable> {
                    KonvoyTomlParser.parseLockfile(project, it)
                }
            } catch (e: Throwable) {
                LOG.warn("Exception parsing konvoy.lock", e)
                null
            }
        }

        LOG.info("Konvoy project synced: ${manifest?.`package`?.name} (kotlin ${manifest?.toolchain?.kotlin})")

        try {
            KonvoyWorkspaceModelUpdater.updateProjectModel(project, manifest!!, lockfile)
        } catch (e: Throwable) {
            LOG.error("Failed to update workspace model", e)
            notifySyncFailed("Failed to update project model: ${e.message}")
            return false
        }

        return true
    }

    private fun notifySyncFailed(message: String) {
        if (project.isDisposed) return
        NotificationGroupManager.getInstance()
            .getNotificationGroup("Konvoy")
            .createNotification("Konvoy sync failed", message, NotificationType.ERROR)
            .addAction(NotificationAction.createSimpleExpiring("Retry") {
                sync()
            })
            .notify(project)
    }

    companion object {
        private val LOG = Logger.getInstance(KonvoyProjectService::class.java)

        fun getInstance(project: Project): KonvoyProjectService =
            project.getService(KonvoyProjectService::class.java)
    }
}
