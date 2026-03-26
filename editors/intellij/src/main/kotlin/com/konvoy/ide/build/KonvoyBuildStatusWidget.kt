package com.konvoy.ide.build

import com.intellij.openapi.components.Service
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.StatusBar
import com.intellij.openapi.wm.StatusBarWidget
import com.intellij.openapi.wm.StatusBarWidgetFactory
import com.intellij.openapi.wm.WindowManager
import com.intellij.util.Consumer
import com.konvoy.ide.sync.KonvoyProjectService
import java.awt.event.MouseEvent

/**
 * Status bar widget that shows build status:
 * - "Konvoy: Building..." during a background build
 * - "Konvoy: 2 errors, 1 warning" after a failed build
 * - "Konvoy: OK" after a successful build
 * - Hidden when build-on-save is disabled
 */
class KonvoyBuildStatusWidgetFactory : StatusBarWidgetFactory {
    override fun getId(): String = KonvoyBuildStatusWidget.ID
    override fun getDisplayName(): String = "Konvoy Build Status"
    override fun isAvailable(project: Project): Boolean =
        KonvoyProjectService.getInstance(project).isKonvoyProject

    override fun createWidget(project: Project): StatusBarWidget =
        KonvoyBuildStatusWidget(project)
}

class KonvoyBuildStatusWidget(private val project: Project) : StatusBarWidget, StatusBarWidget.TextPresentation {

    private var statusBar: StatusBar? = null

    override fun ID(): String = ID

    override fun install(statusBar: StatusBar) {
        this.statusBar = statusBar
    }

    override fun getPresentation(): StatusBarWidget.WidgetPresentation = this

    override fun getText(): String {
        val service = KonvoyBuildStatusService.getInstance(project)
        return service.statusText
    }

    override fun getTooltipText(): String = "Konvoy background build status"

    override fun getAlignment(): Float = 0f

    override fun getClickConsumer(): Consumer<MouseEvent>? = null

    override fun dispose() {
        statusBar = null
    }

    fun update() {
        statusBar?.updateWidget(ID)
    }

    companion object {
        const val ID = "KonvoyBuildStatus"
    }
}

/**
 * Project-level service that holds the current build status text.
 * Updated by [KonvoyBackgroundBuilder], read by [KonvoyBuildStatusWidget].
 */
@Service(Service.Level.PROJECT)
class KonvoyBuildStatusService(private val project: Project) {

    var statusText: String = ""
        private set

    fun setBuilding() {
        statusText = "Konvoy: Building..."
        updateWidget()
    }

    fun setResult(errors: Int, warnings: Int) {
        statusText = when {
            errors > 0 && warnings > 0 -> "Konvoy: $errors error(s), $warnings warning(s)"
            errors > 0 -> "Konvoy: $errors error(s)"
            warnings > 0 -> "Konvoy: $warnings warning(s)"
            else -> "Konvoy: OK"
        }
        updateWidget()
    }

    fun clear() {
        statusText = ""
        updateWidget()
    }

    private fun updateWidget() {
        val statusBar = WindowManager.getInstance().getStatusBar(project) ?: return
        val widget = statusBar.getWidget(KonvoyBuildStatusWidget.ID) as? KonvoyBuildStatusWidget
        widget?.update()
    }

    companion object {
        fun getInstance(project: Project): KonvoyBuildStatusService =
            project.getService(KonvoyBuildStatusService::class.java)
    }
}
