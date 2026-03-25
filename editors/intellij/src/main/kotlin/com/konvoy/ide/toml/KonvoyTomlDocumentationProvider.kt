package com.konvoy.ide.toml

import com.intellij.lang.documentation.AbstractDocumentationProvider
import com.intellij.psi.PsiElement
import org.toml.lang.psi.TomlKey
import org.toml.lang.psi.TomlKeyValue
import org.toml.lang.psi.TomlTable

/**
 * Provides hover documentation for keys in `konvoy.toml`.
 */
class KonvoyTomlDocumentationProvider : AbstractDocumentationProvider() {

    override fun generateDoc(element: PsiElement?, originalElement: PsiElement?): String? {
        if (element?.containingFile?.name != "konvoy.toml") return null

        val key = element as? TomlKey ?: return null
        val keyName = key.text

        // Check if this is a key in a key-value pair
        val kv = key.parent as? TomlKeyValue
        if (kv != null) {
            val tableName = KonvoyTomlPsiUtils.findContainingTableName(kv) ?: return null
            val keys = KonvoyTomlSchema.keysForSection(tableName) ?: return null
            val info = keys[keyName] ?: return null
            return buildDoc(keyName, tableName, info)
        }

        // Check if this is a table header key
        val table = key.parent?.parent as? TomlTable
        if (table != null) {
            val sectionName = key.text
            return when {
                sectionName == "package" -> "<b>[package]</b><br/>Package metadata: name, kind, version, and entrypoint."
                sectionName == "toolchain" -> "<b>[toolchain]</b><br/>Toolchain versions: Kotlin/Native compiler and optional tools."
                sectionName == "dependencies" -> "<b>[dependencies]</b><br/>Project dependencies — path-based or Maven-based."
                sectionName == "plugins" -> "<b>[plugins]</b><br/>Compiler plugins (e.g., kotlinx-serialization)."
                else -> null
            }
        }

        return null
    }

    private fun buildDoc(key: String, section: String, info: KonvoyTomlSchema.KeyInfo): String {
        val sb = StringBuilder()
        sb.append("<b>$key</b> <i>([$section])</i><br/>")
        sb.append(info.description)
        if (info.required) sb.append("<br/><b>Required</b>")
        if (info.values != null) sb.append("<br/>Values: ${info.values.joinToString(", ") { "<code>$it</code>" }}")
        return sb.toString()
    }
}
