package com.konvoy.ide.toml

import com.intellij.lang.annotation.AnnotationHolder
import com.intellij.lang.annotation.Annotator
import com.intellij.lang.annotation.HighlightSeverity
import com.intellij.psi.PsiElement
import org.toml.lang.psi.*

/**
 * Validates `konvoy.toml` files inline, highlighting errors for:
 * - Unknown keys in known sections
 * - Invalid values (e.g., kind must be "bin" or "lib")
 * - Missing required keys (name in [package], kotlin in [toolchain])
 * - Dependency constraints (path xor version+maven)
 *
 * Mirrors the validation logic from the VS Code extension's `tomlSupport.ts`.
 */
class KonvoyTomlAnnotator : Annotator {

    override fun annotate(element: PsiElement, holder: AnnotationHolder) {
        if (element.containingFile?.name != "konvoy.toml") return

        when (element) {
            is TomlKeyValue -> annotateKeyValue(element, holder)
            is TomlTable -> annotateTable(element, holder)
        }
    }

    private fun annotateTable(table: TomlTable, holder: AnnotationHolder) {
        val segments = table.header.key?.segments ?: return
        val sectionName = segments.joinToString(".") { it.text }

        // Validate that required keys exist in known sections
        when (sectionName) {
            "package" -> {
                if (table.entries.none { it.key.text == "name" }) {
                    holder.newAnnotation(HighlightSeverity.ERROR, "Missing required key: name")
                        .range(table.header)
                        .create()
                }
            }
            "toolchain" -> {
                if (table.entries.none { it.key.text == "kotlin" }) {
                    holder.newAnnotation(HighlightSeverity.ERROR, "Missing required key: kotlin")
                        .range(table.header)
                        .create()
                }
            }
        }

        // Validate dependency sub-tables
        if (sectionName.startsWith("dependencies.")) {
            val entries = table.entries.associate { it.key.text to it }
            val hasPath = entries.containsKey("path")
            val hasVersion = entries.containsKey("version")

            if (!hasPath && !hasVersion) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Dependency must have either \"path\" or \"version\"")
                    .range(table.header)
                    .create()
            } else if (hasPath && hasVersion) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Dependency must have only one of \"path\" or \"version\", not both")
                    .range(table.header)
                    .create()
            }
        }

        // Validate plugin sub-tables
        if (sectionName.startsWith("plugins.")) {
            val entries = table.entries.associate { it.key.text to it }
            if (!entries.containsKey("maven")) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Plugin must have \"maven\" set to a groupId:artifactId coordinate")
                    .range(table.header)
                    .create()
            }
            if (!entries.containsKey("version")) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Plugin must have \"version\" set")
                    .range(table.header)
                    .create()
            }
        }

        if (sectionName == "codegen.openapi") {
            val entries = table.entries.associate { it.key.text to it }
            for (requiredKey in listOf("version", "spec", "base_package")) {
                if (!entries.containsKey(requiredKey)) {
                    holder.newAnnotation(HighlightSeverity.ERROR, "OpenAPI codegen must have \"$requiredKey\" set")
                        .range(table.header)
                        .create()
                }
            }
        }
    }

    private fun annotateKeyValue(kv: TomlKeyValue, holder: AnnotationHolder) {
        val keyName = kv.key.text
        val tableName = KonvoyTomlPsiUtils.findContainingTableName(kv) ?: return

        val knownKeys = KonvoyTomlSchema.keysForSection(tableName)

        // Validate unknown keys in known sections
        if (knownKeys != null && keyName !in knownKeys) {
            holder.newAnnotation(HighlightSeverity.WARNING, "Unknown key \"$keyName\" in [$tableName]")
                .range(kv.key)
                .create()
        }

        // Validate specific values
        val value = (kv.value as? TomlLiteral)?.let { literal ->
            val text = literal.text ?: return@let null
            when {
                text.startsWith("\"") -> text.removeSurrounding("\"")
                text.startsWith("'") -> text.removeSurrounding("'")
                else -> text
            }
        }

        if (tableName == "package" && keyName == "kind" && value != null) {
            if (value != "bin" && value != "lib") {
                holder.newAnnotation(HighlightSeverity.ERROR, "Package kind must be \"bin\" or \"lib\"")
                    .range(kv.value!!)
                    .create()
            }
        }

        if (tableName == "package" && keyName == "name" && value != null) {
            if (value.isEmpty()) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Package name must not be empty")
                    .range(kv.value!!)
                    .create()
            } else if (!value.matches(KonvoyTomlPsiUtils.VALID_NAME_RE)) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Package name must match ^[a-zA-Z_][a-zA-Z0-9_-]*\$")
                    .range(kv.value!!)
                    .create()
            }
        }

        if (tableName == "package" && keyName == "entrypoint" && value != null) {
            // Match VS Code extension: error for bin projects, not just warning
            if (!value.endsWith(".kt")) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Entrypoint must end with .kt")
                    .range(kv.value!!)
                    .create()
            }
        }

        if (tableName == "toolchain" && keyName == "kotlin" && value != null) {
            if (value.isEmpty()) {
                holder.newAnnotation(HighlightSeverity.ERROR, "Kotlin version must not be empty")
                    .range(kv.value!!)
                    .create()
            }
        }

        if (tableName == "codegen.openapi" && value != null) {
            annotateOpenApiValue(keyName, value, kv, holder)
        }
    }

    // Mirrors the Rust manifest validator (validate_codegen) and the VS Code
    // extension so all three agree with `konvoy build`.
    private fun annotateOpenApiValue(
        keyName: String,
        value: String,
        kv: TomlKeyValue,
        holder: AnnotationHolder,
    ) {
        fun error(message: String) {
            holder.newAnnotation(HighlightSeverity.ERROR, message)
                .range(kv.value!!)
                .create()
        }

        // Validate the trimmed value, matching the Rust validator (which trims)
        // and the VS Code extension. Otherwise surrounding whitespace that
        // `konvoy build` accepts would be flagged as a false-positive error.
        val value = value.trim()

        when (keyName) {
            "version" -> {
                val major = value.substringBefore('.').substringBefore('-').substringBefore('+').toIntOrNull()
                when {
                    value.isEmpty() -> error("Fabrikt version must not be empty")
                    major == null -> error("version must be a valid Fabrikt version like \"20.0.0\"")
                    major < MIN_FABRIKT_MAJOR -> error("Konvoy requires Fabrikt $MIN_FABRIKT_MAJOR.0.0 or newer")
                }
            }
            "spec" -> {
                val isAbsolute = value.startsWith("/") || ABSOLUTE_WINDOWS_PATH_RE.containsMatchIn(value)
                when {
                    value.isEmpty() -> error("spec must not be empty")
                    isAbsolute -> error("spec must be a relative path inside the project")
                    value.split('/', '\\').contains("..") ->
                        error("spec must be a relative path inside the project (must not contain \"..\")")
                    !(value.endsWith(".yaml") || value.endsWith(".yml") || value.endsWith(".json")) ->
                        error("spec must point to an OpenAPI .yaml, .yml, or .json file")
                }
            }
            "base_package" -> {
                when {
                    value.isEmpty() -> error("base_package must not be empty")
                    !value.matches(PACKAGE_RE) ->
                        error("base_package must be dot-separated identifiers like \"com.example.api\"")
                }
            }
        }
    }

    private companion object {
        const val MIN_FABRIKT_MAJOR = 18
        val PACKAGE_RE = Regex("^[A-Za-z_][A-Za-z0-9_]*(\\.[A-Za-z_][A-Za-z0-9_]*)*$")
        val ABSOLUTE_WINDOWS_PATH_RE = Regex("^[A-Za-z]:[\\\\/]")
    }
}
