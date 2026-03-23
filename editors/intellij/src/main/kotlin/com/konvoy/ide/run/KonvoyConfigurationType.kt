package com.konvoy.ide.run

import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.execution.configurations.ConfigurationType
import com.intellij.icons.AllIcons
import javax.swing.Icon

/**
 * Registers Konvoy run configuration type in IntelliJ's run configuration UI.
 * Provides factories for build, run, test, and lint configurations.
 */
class KonvoyConfigurationType : ConfigurationType {
    override fun getDisplayName(): String = "Konvoy"
    override fun getConfigurationTypeDescription(): String = "Konvoy build tool commands"
    override fun getIcon(): Icon = AllIcons.Actions.Execute
    override fun getId(): String = ID

    override fun getConfigurationFactories(): Array<ConfigurationFactory> = arrayOf(
        KonvoyRunFactory(this),
        KonvoyBuildFactory(this),
        KonvoyTestFactory(this),
        KonvoyLintFactory(this),
    )

    companion object {
        const val ID = "KonvoyConfigurationType"
    }
}
