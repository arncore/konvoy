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
    var target: KonvoyTarget = KonvoyTarget.HOST
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
        element.setAttribute("konvoy-target", target.name)
        element.setAttribute("konvoy-extra-args", extraArgs)
    }

    override fun readExternal(element: Element) {
        super.readExternal(element)
        command = element.getAttributeValue("konvoy-command")
            ?.let { name -> KonvoyCommand.entries.find { it.name == name } }
            ?: KonvoyCommand.RUN
        target = element.getAttributeValue("konvoy-target")
            ?.let { name -> KonvoyTarget.entries.find { it.name == name } }
            ?: KonvoyTarget.HOST
        extraArgs = element.getAttributeValue("konvoy-extra-args") ?: ""
    }
}

enum class KonvoyCommand(val displayName: String, val subcommand: String) {
    BUILD("Build", "build"),
    RUN("Run", "run"),
    TEST("Test", "test"),
    LINT("Lint", "lint"),
}

internal val KonvoyCommand.supportsTarget: Boolean
    get() = this != KonvoyCommand.LINT

enum class KonvoyTarget(val displayName: String, val cliValue: String?) {
    HOST("Host", null),
    LINUX_X64("linux_x64", "linux_x64"),
    LINUX_ARM64("linux_arm64", "linux_arm64"),
    MACOS_X64("macos_x64", "macos_x64"),
    MACOS_ARM64("macos_arm64", "macos_arm64");

    override fun toString(): String = displayName
}

internal fun createKonvoyCommandLine(
    workDirectory: File,
    command: KonvoyCommand,
    target: KonvoyTarget,
    extraArgs: String,
): GeneralCommandLine {
    val cmd = GeneralCommandLine("konvoy", command.subcommand)
    if (command.supportsTarget && target.cliValue != null) {
        cmd.addParameters("--target", target.cliValue)
    }
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
            config.target,
            config.extraArgs,
        )
        return ProcessHandlerFactory.getInstance().createColoredProcessHandler(cmd)
    }
}
