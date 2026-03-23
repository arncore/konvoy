package com.konvoy.ide.sync

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.project.Project
import com.intellij.openapi.startup.ProjectActivity

/**
 * Detects konvoy.toml on project open and triggers initial sync.
 */
class KonvoyStartupActivity : ProjectActivity {
    override suspend fun execute(project: Project) {
        val service = KonvoyProjectService.getInstance(project)
        if (!service.isKonvoyProject) return

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
