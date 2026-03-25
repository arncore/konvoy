package com.konvoy.ide.toml

import com.intellij.codeInsight.completion.*
import com.intellij.codeInsight.lookup.LookupElementBuilder
import com.intellij.icons.AllIcons
import com.intellij.patterns.PlatformPatterns
import com.intellij.psi.PsiElement
import com.intellij.util.ProcessingContext
import org.toml.lang.psi.*

/**
 * Provides auto-completion in `konvoy.toml` files:
 * - Section headers ([package], [toolchain], [dependencies], [plugins])
 * - Keys within sections (name, kind, kotlin, etc.)
 * - Values for known keys (bin/lib for kind)
 */
class KonvoyTomlCompletionContributor : CompletionContributor() {

    init {
        // Complete keys inside tables
        extend(
            CompletionType.BASIC,
            PlatformPatterns.psiElement().withParent(TomlKey::class.java),
            KeyCompletionProvider(),
        )

        // Complete values for known keys
        extend(
            CompletionType.BASIC,
            PlatformPatterns.psiElement().withParent(TomlLiteral::class.java),
            ValueCompletionProvider(),
        )
    }

    private fun isKonvoyToml(element: PsiElement): Boolean {
        return element.containingFile?.name == "konvoy.toml"
    }

    private inner class KeyCompletionProvider : CompletionProvider<CompletionParameters>() {
        override fun addCompletions(
            parameters: CompletionParameters,
            context: ProcessingContext,
            result: CompletionResultSet,
        ) {
            val position = parameters.position
            if (!isKonvoyToml(position)) return

            val tableName = KonvoyTomlPsiUtils.findContainingTableName(position)

            if (tableName == null) {
                // At top level — suggest section headers
                for (section in KonvoyTomlSchema.SECTIONS) {
                    result.addElement(
                        LookupElementBuilder.create(section)
                            .withIcon(AllIcons.Nodes.Tag)
                            .withTypeText("section")
                    )
                }
                return
            }

            // Inside a table — suggest keys for this section
            val keys = KonvoyTomlSchema.keysForSection(tableName) ?: return

            // Filter out keys that already exist in this table
            val existingKeys = findExistingKeys(position)

            for ((key, info) in keys) {
                if (key in existingKeys) continue
                val element = LookupElementBuilder.create(key)
                    .withIcon(AllIcons.Nodes.Property)
                    .withTypeText(if (info.required) "required" else "optional")
                    .withTailText(if (info.values != null) " (${info.values.joinToString("|")})" else "")
                result.addElement(element)
            }
        }

        private fun findExistingKeys(position: PsiElement): Set<String> {
            var current = position.parent
            while (current != null) {
                if (current is TomlTable) {
                    return current.entries.map { it.key.text }.toSet()
                }
                current = current.parent
            }
            return emptySet()
        }
    }

    private inner class ValueCompletionProvider : CompletionProvider<CompletionParameters>() {
        override fun addCompletions(
            parameters: CompletionParameters,
            context: ProcessingContext,
            result: CompletionResultSet,
        ) {
            val position = parameters.position
            if (!isKonvoyToml(position)) return

            // Find the key this value belongs to
            val entry = findContainingEntry(position) ?: return
            val keyName = entry.key.text
            val tableName = KonvoyTomlPsiUtils.findContainingTableName(position) ?: return

            val keys = KonvoyTomlSchema.keysForSection(tableName) ?: return
            val info = keys[keyName] ?: return
            val values = info.values ?: return

            for (value in values) {
                result.addElement(
                    LookupElementBuilder.create(value)
                        .withIcon(AllIcons.Nodes.Enum)
                )
            }
        }

        private fun findContainingEntry(element: PsiElement): TomlKeyValue? {
            var current = element.parent
            while (current != null) {
                if (current is TomlKeyValue) return current
                current = current.parent
            }
            return null
        }
    }
}
