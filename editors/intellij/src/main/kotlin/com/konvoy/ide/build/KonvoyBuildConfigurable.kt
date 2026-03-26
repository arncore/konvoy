package com.konvoy.ide.build

import com.intellij.openapi.options.BoundSearchableConfigurable
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.DialogPanel
import com.intellij.ui.dsl.builder.bindSelected
import com.intellij.ui.dsl.builder.panel

/**
 * Settings page at Settings > Konvoy for project-level build options.
 */
class KonvoyBuildConfigurable(private val project: Project) :
    BoundSearchableConfigurable("Konvoy", "konvoy.settings") {

    override fun createPanel(): DialogPanel {
        val settings = KonvoyBuildSettings.getInstance(project)
        return panel {
            group("Build") {
                row {
                    checkBox("Build on save")
                        .comment("Automatically run konvoy build when saving .kt files. Compiler errors will appear as inline markers.")
                        .bindSelected(settings.state::buildOnSave)
                }
            }
        }
    }
}
