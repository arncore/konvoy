package com.konvoy.ide.run

import com.intellij.execution.actions.ConfigurationContext
import com.intellij.execution.actions.LazyRunConfigurationProducer
import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.openapi.util.Ref
import com.intellij.psi.PsiElement
import com.konvoy.ide.sync.KonvoyProjectService

/**
 * Automatically creates "konvoy run" configurations from context.
 * When a user right-clicks a Kotlin file containing a `fun main()`,
 * this producer offers a Konvoy run configuration.
 */
class KonvoyRunConfigurationProducer : LazyRunConfigurationProducer<KonvoyRunConfiguration>() {

    override fun getConfigurationFactory(): ConfigurationFactory =
        KonvoyConfigurationType().configurationFactories.first() // KonvoyRunFactory

    override fun setupConfigurationFromContext(
        configuration: KonvoyRunConfiguration,
        context: ConfigurationContext,
        sourceElement: Ref<PsiElement>,
    ): Boolean {
        val project = context.project
        val service = KonvoyProjectService.getInstance(project)
        if (!service.isKonvoyProject) return false

        val manifest = service.manifest ?: return false
        configuration.name = "konvoy run ${manifest.`package`.name}"
        configuration.command = KonvoyCommand.RUN
        return true
    }

    override fun isConfigurationFromContext(
        configuration: KonvoyRunConfiguration,
        context: ConfigurationContext,
    ): Boolean {
        val project = context.project
        val service = KonvoyProjectService.getInstance(project)
        return service.isKonvoyProject && configuration.command == KonvoyCommand.RUN
    }
}
