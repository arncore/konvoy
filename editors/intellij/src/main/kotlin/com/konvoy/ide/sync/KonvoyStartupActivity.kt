package com.konvoy.ide.sync

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.fileEditor.FileDocumentManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity
import com.konvoy.ide.build.KonvoyBuildOnSaveListener

/**
 * Detects konvoy.toml on project open and triggers initial sync.
 * Also registers the build-on-save listener scoped to this project.
 */
class KonvoyStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        val service = KonvoyProjectService.getInstance(project)
        if (!service.isKonvoyProject) return

        service.sync()

        // Register build-on-save listener scoped to this project's lifetime.
        // Using the project's message bus ensures auto-disposal on project close.
        project.messageBus.connect(project).subscribe(
            FileDocumentManagerListener.TOPIC,
            KonvoyBuildOnSaveListener(project),
        )

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
