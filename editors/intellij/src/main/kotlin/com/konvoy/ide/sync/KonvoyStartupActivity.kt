package com.konvoy.ide.sync

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.fileEditor.FileDocumentManagerListener
import com.intellij.openapi.project.DumbService
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.startup.StartupManager
import com.konvoy.ide.build.KonvoyBuildOnSaveListener

/**
 * Detects konvoy.toml on project open and triggers the initial sync.
 * Also registers the build-on-save listener scoped to this project.
 */
class KonvoyStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        val service = KonvoyProjectService.getInstance(project)
        if (!service.isKonvoyProject) return

        // Register the build-on-save listener immediately (cheap, and scoped to
        // this project's lifetime via the message bus connection).
        project.messageBus.connect(project).subscribe(
            FileDocumentManagerListener.TOPIC,
            KonvoyBuildOnSaveListener(project),
        )

        // Defer the initial sync until the project is fully opened AND indexing
        // has settled. Running it during project open races with IntelliJ's own
        // workspace-model cache restore ("sync real project state into workspace
        // model"), which runs slightly later and reverts the libraries/source
        // roots we just added — forcing the user to sync manually. Running after
        // open + in smart mode lands our changes after that reconcile, so they
        // stick on the very first open.
        StartupManager.getInstance(project).runAfterOpened {
            DumbService.getInstance(project).runWhenSmart {
                if (project.isDisposed) return@runWhenSmart
                service.sync()

                NotificationGroupManager.getInstance()
                    .getNotificationGroup("Konvoy")
                    .createNotification(
                        "Konvoy project detected",
                        "Synced project model from konvoy.toml",
                        NotificationType.INFORMATION,
                    )
                    .notify(project)
            }
        }
    }
}
