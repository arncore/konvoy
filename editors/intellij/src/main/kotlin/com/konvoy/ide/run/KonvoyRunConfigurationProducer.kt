package com.konvoy.ide.run

import com.intellij.execution.actions.ConfigurationContext
import com.intellij.execution.actions.LazyRunConfigurationProducer
import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.openapi.util.Ref
import com.intellij.psi.PsiElement
import com.konvoy.ide.config.PackageKind
import com.konvoy.ide.sync.KonvoyProjectService

/**
 * Automatically creates Konvoy run configurations from context.
 *
 * - Gutter icon or right-click on `fun main()` → `konvoy run`
 * - Gutter icon or right-click on `@Test fun` in src/test/ → `konvoy test --filter=<name>`
 * - Right-click in a bin project → `konvoy run`
 */
class KonvoyRunConfigurationProducer : LazyRunConfigurationProducer<KonvoyRunConfiguration>() {

    override fun getConfigurationFactory(): ConfigurationFactory =
        KonvoyConfigurationType().configurationFactories.first()

    override fun setupConfigurationFromContext(
        configuration: KonvoyRunConfiguration,
        context: ConfigurationContext,
        sourceElement: Ref<PsiElement>,
    ): Boolean {
        val project = context.project
        val service = KonvoyProjectService.getInstance(project)
        if (!service.isKonvoyProject) return false

        val manifest = service.manifest ?: return false
        val element = sourceElement.get()

        // Check if we're on a test function
        val testFunction = KonvoyPsiUtils.findTestFunction(element, project)
        if (testFunction != null) {
            val testName = testFunction.name ?: return false
            configuration.name = "konvoy test $testName"
            configuration.command = KonvoyCommand.TEST
            configuration.extraArgs = "--filter=$testName"
            return true
        }

        // Check if we're on fun main()
        val mainFunction = KonvoyPsiUtils.findMainFunction(element, project)
        if (mainFunction != null) {
            configuration.name = "konvoy run ${manifest.`package`.name}"
            configuration.command = KonvoyCommand.RUN
            return true
        }

        // Default: offer konvoy run for bin projects only
        if (manifest.`package`.kind != PackageKind.BIN) return false
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
        if (!service.isKonvoyProject) return false

        val element = context.psiLocation ?: return false

        val testFunction = KonvoyPsiUtils.findTestFunction(element, project)
        if (testFunction != null) {
            val testName = testFunction.name ?: return false
            return configuration.command == KonvoyCommand.TEST &&
                configuration.extraArgs.contains(testName)
        }

        val mainFunction = KonvoyPsiUtils.findMainFunction(element, project)
        if (mainFunction != null) {
            return configuration.command == KonvoyCommand.RUN
        }

        return configuration.command == KonvoyCommand.RUN
    }
}
