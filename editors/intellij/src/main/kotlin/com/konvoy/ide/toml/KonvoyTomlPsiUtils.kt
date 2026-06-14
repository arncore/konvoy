package com.konvoy.ide.toml

import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.psi.util.PsiTreeUtil
import org.toml.lang.psi.TomlTable

/**
 * Shared PSI utilities for konvoy.toml providers.
 */
object KonvoyTomlPsiUtils {

    /** Walks up the PSI tree to find the containing TOML table name (e.g., "package", "dependencies.foo"). */
    fun findContainingTableName(element: PsiElement): String? {
        var current = element.parent
        while (current != null) {
            if (current is TomlTable) {
                return current.header.key?.segments?.joinToString(".") { it.text }
            }
            current = current.parent
        }
        return null
    }

    /**
     * Locate the PSI element a dotted key path points at — used to place a diagnostic
     * from `konvoy check` on the right spot in `konvoy.toml`.
     *
     * Resolves the longest table-header prefix (so `dependencies.foo` matches either a
     * `[dependencies.foo]` sub-table or the `foo` key in `[dependencies]`, and
     * `package.name` matches the `name` key in `[package]`), then the remaining key
     * within that table. Returns the table header when the path *is* a table, the key
     * element when it's a key in a table, or null when nothing matches.
     */
    fun findElementByKeyPath(file: PsiFile, keyPath: String): PsiElement? {
        var bestTable: TomlTable? = null
        var bestHeader = ""
        for (table in PsiTreeUtil.findChildrenOfType(file, TomlTable::class.java)) {
            val header = table.header.key?.segments?.joinToString(".") { it.text } ?: continue
            val isPrefix = keyPath == header || keyPath.startsWith("$header.")
            if (isPrefix && header.length > bestHeader.length) {
                bestTable = table
                bestHeader = header
            }
        }
        val table = bestTable ?: return null
        val remainder = keyPath.removePrefix(bestHeader).removePrefix(".")
        if (remainder.isEmpty()) return table.header
        val keyName = remainder.substringBefore(".")
        return table.entries.firstOrNull { it.key.text == keyName }?.key ?: table.header
    }

    /** Valid package name pattern matching the Rust config crate. */
    val VALID_NAME_RE = Regex("^[a-zA-Z_][a-zA-Z0-9_-]*$")
}
