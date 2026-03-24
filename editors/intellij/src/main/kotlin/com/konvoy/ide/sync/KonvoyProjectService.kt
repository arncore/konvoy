package com.konvoy.ide.sync

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
     */
    fun sync() {
        val manifestFile = findManifestFile()
        if (manifestFile == null) {
            LOG.info("No konvoy.toml found in ${project.basePath}, skipping sync")
            manifest = null
            lockfile = null
            return
        }

        manifest = ReadAction.compute<KonvoyManifest?, Throwable> {
            KonvoyTomlParser.parseManifest(project, manifestFile)
        }
        if (manifest == null) {
            LOG.warn("Failed to parse konvoy.toml")
            return
        }

        val lockfileFile = findLockfile()
        lockfile = lockfileFile?.let {
            ReadAction.compute<KonvoyLockfile?, Throwable> {
                KonvoyTomlParser.parseLockfile(project, it)
            }
        }

        LOG.info("Konvoy project synced: ${manifest?.`package`?.name} (kotlin ${manifest?.toolchain?.kotlin})")

        KonvoyWorkspaceModelUpdater.updateProjectModel(project, manifest!!, lockfile)
    }

    companion object {
        private val LOG = Logger.getInstance(KonvoyProjectService::class.java)

        fun getInstance(project: Project): KonvoyProjectService =
            project.getService(KonvoyProjectService::class.java)
    }
}
