package com.konvoy.ide.run

import com.intellij.execution.configurations.GeneralCommandLine
import junit.framework.TestCase
import java.io.File

class KonvoyRunConfigurationCommandLineTest : TestCase() {

    fun testCommandLineRequestsPlainProgressForRunConsole() {
        val cmd = createKonvoyCommandLine(File("/tmp/project"), KonvoyCommand.RUN, "")

        assertEquals("plain", cmd.environment["KONVOY_PROGRESS"])
    }

    fun testCommandLineUsesConsoleParentEnvironment() {
        val cmd = createKonvoyCommandLine(File("/tmp/project"), KonvoyCommand.TEST, "")

        assertEquals(
            GeneralCommandLine.ParentEnvironmentType.CONSOLE,
            cmd.parentEnvironmentType,
        )
    }

    fun testCommandLinePreservesCommandAndExtraArgs() {
        val cmd = createKonvoyCommandLine(
            File("/tmp/project"),
            KonvoyCommand.TEST,
            "--filter \"OtherTest.i am a test\"",
        )

        assertEquals("/tmp/project", cmd.workDirectory.path)
        assertEquals(listOf("test", "--filter", "OtherTest.i am a test"), cmd.parametersList.parameters)
    }
}
