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
    private val targetCombo = ComboBox(KonvoyTarget.entries.toTypedArray())
    private val extraArgsField = JTextField()

    init {
        commandCombo.addActionListener {
            updateTargetSelector()
        }
        updateTargetSelector()
    }

    override fun createEditor(): JComponent = panel {
        row("Command:") {
            cell(commandCombo)
        }
        row("Target:") {
            cell(targetCombo)
        }
        row("Extra arguments:") {
            cell(extraArgsField).resizableColumn()
        }
    }

    override fun applyEditorTo(config: KonvoyRunConfiguration) {
        config.command = commandCombo.selectedItem as KonvoyCommand
        config.target = targetCombo.selectedItem as KonvoyTarget
        config.extraArgs = extraArgsField.text
    }

    override fun resetEditorFrom(config: KonvoyRunConfiguration) {
        commandCombo.selectedItem = config.command
        targetCombo.selectedItem = config.target
        extraArgsField.text = config.extraArgs
        updateTargetSelector()
    }

    private fun updateTargetSelector() {
        val selectedCommand = commandCombo.selectedItem as? KonvoyCommand ?: KonvoyCommand.RUN
        targetCombo.isEnabled = isTargetSelectorEnabled(selectedCommand)
    }
}

internal fun isTargetSelectorEnabled(command: KonvoyCommand): Boolean =
    command.supportsTarget
