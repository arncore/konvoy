package com.konvoy.ide.run

import com.intellij.execution.configurations.GeneralCommandLine
import junit.framework.TestCase
import java.io.File

class KonvoyRunConfigurationCommandLineTest : TestCase() {

    fun testCommandLineRequestsPlainProgressForRunConsole() {
        val cmd = createKonvoyCommandLine(File("/tmp/project"), KonvoyCommand.RUN, KonvoyTarget.HOST, "")

        assertEquals("plain", cmd.environment["KONVOY_PROGRESS"])
    }

    fun testCommandLineUsesConsoleParentEnvironment() {
        val cmd = createKonvoyCommandLine(File("/tmp/project"), KonvoyCommand.TEST, KonvoyTarget.HOST, "")

        assertEquals(
            GeneralCommandLine.ParentEnvironmentType.CONSOLE,
            cmd.parentEnvironmentType,
        )
    }

    fun testCommandLinePreservesCommandAndExtraArgs() {
        val cmd = createKonvoyCommandLine(
            File("/tmp/project"),
            KonvoyCommand.TEST,
            KonvoyTarget.HOST,
            "--filter \"OtherTest.i am a test\"",
        )

        assertEquals("/tmp/project", cmd.workDirectory.path)
        assertEquals(listOf("test", "--filter", "OtherTest.i am a test"), cmd.parametersList.parameters)
    }

    fun testCommandLineAddsSpecificTargetBeforeExtraArgs() {
        val cmd = createKonvoyCommandLine(
            File("/tmp/project"),
            KonvoyCommand.TEST,
            KonvoyTarget.MACOS_ARM64,
            "--filter MainTest.*",
        )

        assertEquals(
            listOf("test", "--target", "macos_arm64", "--filter", "MainTest.*"),
            cmd.parametersList.parameters,
        )
    }

    fun testCommandLineOmitsHostTarget() {
        val cmd = createKonvoyCommandLine(File("/tmp/project"), KonvoyCommand.BUILD, KonvoyTarget.HOST, "")

        assertEquals(listOf("build"), cmd.parametersList.parameters)
    }

    fun testCommandLineDoesNotAddTargetForLint() {
        val cmd = createKonvoyCommandLine(
            File("/tmp/project"),
            KonvoyCommand.LINT,
            KonvoyTarget.LINUX_X64,
            "--verbose",
        )

        assertEquals(listOf("lint", "--verbose"), cmd.parametersList.parameters)
    }

    fun testCommandLineDoesNotAddTargetForGenerate() {
        val cmd = createKonvoyCommandLine(
            File("/tmp/project"),
            KonvoyCommand.GENERATE,
            KonvoyTarget.LINUX_X64,
            "--verbose",
        )

        assertEquals(listOf("generate", "--verbose"), cmd.parametersList.parameters)
    }

    fun testTargetSelectorIsOnlyEnabledForCommandsThatAcceptTargets() {
        assertTrue(isTargetSelectorEnabled(KonvoyCommand.BUILD))
        assertTrue(isTargetSelectorEnabled(KonvoyCommand.RUN))
        assertTrue(isTargetSelectorEnabled(KonvoyCommand.TEST))
        assertFalse(isTargetSelectorEnabled(KonvoyCommand.LINT))
        assertFalse(isTargetSelectorEnabled(KonvoyCommand.GENERATE))
    }
}
