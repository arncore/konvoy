package com.konvoy.ide.toml

import com.intellij.psi.PsiElement
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

    /** Valid package name pattern matching the Rust config crate. */
    val VALID_NAME_RE = Regex("^[a-zA-Z_][a-zA-Z0-9_-]*$")
}
