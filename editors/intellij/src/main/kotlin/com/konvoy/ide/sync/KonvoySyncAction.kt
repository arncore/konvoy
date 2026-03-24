package com.konvoy.ide.sync

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent

/**
 * Manual action to re-sync the Konvoy project model.
 * Available in Build menu and can be assigned a keyboard shortcut.
 */
class KonvoySyncAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val service = KonvoyProjectService.getInstance(project)
        service.sync()

        NotificationGroupManager.getInstance()
            .getNotificationGroup("Konvoy")
            .createNotification(
                "Konvoy sync complete",
                "Project model refreshed from konvoy.toml",
                NotificationType.INFORMATION,
            )
            .notify(project)
    }

    override fun update(e: AnActionEvent) {
        val project = e.project
        e.presentation.isEnabledAndVisible =
            project != null && KonvoyProjectService.getInstance(project).isKonvoyProject
    }
}
