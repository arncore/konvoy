package com.konvoy.ide.sync

import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.progress.ProgressIndicator
import com.intellij.openapi.progress.ProgressManager
import com.intellij.openapi.progress.Task
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * Build-menu action that runs `konvoy generate` and refreshes the project so freshly
 * generated sources are picked up (and indexed via the generated source roots).
 *
 * A thin wrapper over the CLI — no generation logic lives here. If codegen isn't
 * configured, `konvoy generate` says so and that message is surfaced as the
 * notification body.
 */
class KonvoyGenerateAction : AnAction() {

    private data class Result(val ok: Boolean, val output: String)

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val basePath = project.basePath ?: return

        ProgressManager.getInstance().run(
            object : Task.Backgroundable(project, "Running konvoy generate", true) {
                override fun run(indicator: ProgressIndicator) {
                    val result = runGenerate(File(basePath))
                    ApplicationManager.getApplication().invokeLater {
                        if (project.isDisposed) return@invokeLater
                        if (result.ok) {
                            // Bring the new sources into the VFS and re-sync so they
                            // register as (generated) source roots and get indexed.
                            LocalFileSystem.getInstance()
                                .refreshAndFindFileByPath("$basePath/.konvoy/gen")
                            KonvoyProjectService.getInstance(project).sync()
                        }
                        notify(project, result)
                    }
                }
            },
        )
    }

    private fun runGenerate(dir: File): Result {
        return try {
            val process = ProcessBuilder("konvoy", "generate")
                .directory(dir)
                .redirectErrorStream(true)
                .start()
            val output = process.inputStream.bufferedReader().readText()
            if (!process.waitFor(120, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                return Result(false, "konvoy generate timed out")
            }
            Result(process.exitValue() == 0, output)
        } catch (e: Exception) {
            Result(false, e.message ?: "failed to run konvoy generate")
        }
    }

    private fun notify(project: Project, result: Result) {
        val type = if (result.ok) NotificationType.INFORMATION else NotificationType.ERROR
        val title = if (result.ok) "Konvoy generate complete" else "Konvoy generate failed"
        NotificationGroupManager.getInstance()
            .getNotificationGroup("Konvoy")
            .createNotification(title, result.output.trim().ifEmpty { "(no output)" }, type)
            .notify(project)
    }

    override fun update(e: AnActionEvent) {
        val project = e.project
        e.presentation.isEnabledAndVisible =
            project != null && KonvoyProjectService.getInstance(project).isKonvoyProject
    }
}
