package com.konvoy.ide.run

import com.intellij.execution.Executor
import com.intellij.execution.configurations.*
import com.intellij.execution.process.ProcessHandler
import com.intellij.execution.process.ProcessHandlerFactory
import com.intellij.execution.runners.ExecutionEnvironment
import com.intellij.openapi.options.SettingsEditor
import com.intellij.openapi.project.Project
import org.jdom.Element
import java.io.File

/**
 * A run configuration that executes a `konvoy` subcommand.
 */
class KonvoyRunConfiguration(
    project: Project,
    factory: ConfigurationFactory,
    name: String,
) : RunConfigurationBase<RunConfigurationOptions>(project, factory, name) {

    var command: KonvoyCommand = KonvoyCommand.RUN
    var extraArgs: String = ""

    override fun getConfigurationEditor(): SettingsEditor<out RunConfiguration> =
        KonvoySettingsEditor()

    override fun checkConfiguration() {
        if (project.basePath == null) {
            throw RuntimeConfigurationError("Project base path not found")
        }
        val manifestFile = File(project.basePath!!, "konvoy.toml")
        if (!manifestFile.exists()) {
            throw RuntimeConfigurationError("No konvoy.toml found in project root")
        }
    }

    override fun getState(executor: Executor, environment: ExecutionEnvironment): RunProfileState {
        return KonvoyCommandLineState(environment, this)
    }

    override fun writeExternal(element: Element) {
        super.writeExternal(element)
        element.setAttribute("konvoy-command", command.name)
        element.setAttribute("konvoy-extra-args", extraArgs)
    }

    override fun readExternal(element: Element) {
        super.readExternal(element)
        command = element.getAttributeValue("konvoy-command")
            ?.let { name -> KonvoyCommand.entries.find { it.name == name } }
            ?: KonvoyCommand.RUN
        extraArgs = element.getAttributeValue("konvoy-extra-args") ?: ""
    }
}

enum class KonvoyCommand(val displayName: String, val subcommand: String) {
    BUILD("Build", "build"),
    RUN("Run", "run"),
    TEST("Test", "test"),
    LINT("Lint", "lint"),
}

internal fun createKonvoyCommandLine(
    workDirectory: File,
    command: KonvoyCommand,
    extraArgs: String,
): GeneralCommandLine {
    val cmd = GeneralCommandLine("konvoy", command.subcommand)
    if (extraArgs.isNotBlank()) {
        cmd.addParameters(ParametersList.parse(extraArgs).toList())
    }
    cmd.workDirectory = workDirectory
    cmd.withParentEnvironmentType(GeneralCommandLine.ParentEnvironmentType.CONSOLE)
    cmd.withEnvironment("KONVOY_PROGRESS", "plain")
    return cmd
}

/**
 * Executes the konvoy command as an OS process.
 */
class KonvoyCommandLineState(
    environment: ExecutionEnvironment,
    private val config: KonvoyRunConfiguration,
) : CommandLineState(environment) {

    override fun startProcess(): ProcessHandler {
        val cmd = createKonvoyCommandLine(
            File(config.project.basePath!!),
            config.command,
            config.extraArgs,
        )
        return ProcessHandlerFactory.getInstance().createColoredProcessHandler(cmd)
    }
}
