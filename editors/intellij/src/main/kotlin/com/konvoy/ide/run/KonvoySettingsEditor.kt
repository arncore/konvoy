package com.konvoy.ide.run

import com.intellij.openapi.options.SettingsEditor
import com.intellij.openapi.ui.ComboBox
import com.intellij.ui.dsl.builder.panel
import javax.swing.JComponent
import javax.swing.JTextField

/**
 * UI editor for Konvoy run configuration settings.
 */
class KonvoySettingsEditor : SettingsEditor<KonvoyRunConfiguration>() {
    private val commandCombo = ComboBox(KonvoyCommand.entries.toTypedArray())
    private val extraArgsField = JTextField()

    override fun createEditor(): JComponent = panel {
        row("Command:") {
            cell(commandCombo)
        }
        row("Extra arguments:") {
            cell(extraArgsField).resizableColumn()
        }
    }

    override fun applyEditorTo(config: KonvoyRunConfiguration) {
        config.command = commandCombo.selectedItem as KonvoyCommand
        config.extraArgs = extraArgsField.text
    }

    override fun resetEditorFrom(config: KonvoyRunConfiguration) {
        commandCombo.selectedItem = config.command
        extraArgsField.text = config.extraArgs
    }
}
