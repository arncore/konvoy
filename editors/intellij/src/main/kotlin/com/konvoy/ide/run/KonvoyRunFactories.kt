package com.konvoy.ide.run

import com.intellij.execution.configurations.ConfigurationFactory
import com.intellij.execution.configurations.RunConfiguration
import com.intellij.openapi.project.Project

class KonvoyRunFactory(type: KonvoyConfigurationType) : ConfigurationFactory(type) {
    override fun getId(): String = "KonvoyRun"
    override fun getName(): String = "Run"
    override fun createTemplateConfiguration(project: Project): RunConfiguration =
        KonvoyRunConfiguration(project, this, "konvoy run").also { it.command = KonvoyCommand.RUN }
}

class KonvoyBuildFactory(type: KonvoyConfigurationType) : ConfigurationFactory(type) {
    override fun getId(): String = "KonvoyBuild"
    override fun getName(): String = "Build"
    override fun createTemplateConfiguration(project: Project): RunConfiguration =
        KonvoyRunConfiguration(project, this, "konvoy build").also { it.command = KonvoyCommand.BUILD }
}

class KonvoyTestFactory(type: KonvoyConfigurationType) : ConfigurationFactory(type) {
    override fun getId(): String = "KonvoyTest"
    override fun getName(): String = "Test"
    override fun createTemplateConfiguration(project: Project): RunConfiguration =
        KonvoyRunConfiguration(project, this, "konvoy test").also { it.command = KonvoyCommand.TEST }
}

class KonvoyLintFactory(type: KonvoyConfigurationType) : ConfigurationFactory(type) {
    override fun getId(): String = "KonvoyLint"
    override fun getName(): String = "Lint"
    override fun createTemplateConfiguration(project: Project): RunConfiguration =
        KonvoyRunConfiguration(project, this, "konvoy lint").also { it.command = KonvoyCommand.LINT }
}
