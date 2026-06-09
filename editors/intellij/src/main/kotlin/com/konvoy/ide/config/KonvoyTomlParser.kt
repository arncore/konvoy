package com.konvoy.ide.config

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.vfs.VirtualFile
import org.toml.lang.psi.*
import com.intellij.psi.PsiManager
import com.intellij.openapi.project.Project

/**
 * Parses `konvoy.toml` and `konvoy.lock` files using the TOML PSI.
 *
 * Requires the IntelliJ TOML plugin: when a file is not recognized as TOML (or
 * PSI parsing throws), parsing returns null rather than falling back to a
 * hand-rolled line parser. `konvoy.lock` is associated with the TOML file type
 * in plugin.xml so it parses via PSI like `konvoy.toml`.
 */
object KonvoyTomlParser {
    private val LOG = Logger.getInstance(KonvoyTomlParser::class.java)

    fun parseManifest(project: Project, file: VirtualFile): KonvoyManifest? {
        val psi = PsiManager.getInstance(project).findFile(file) as? TomlFile
        if (psi == null) {
            LOG.warn("konvoy.toml is not recognized as a TOML file; cannot parse")
            return null
        }
        return try {
            parseManifestFromPsi(psi)
        } catch (e: Exception) {
            LOG.warn("Failed to parse konvoy.toml", e)
            null
        }
    }

    fun parseLockfile(project: Project, file: VirtualFile): KonvoyLockfile? {
        val psi = PsiManager.getInstance(project).findFile(file) as? TomlFile
        if (psi == null) {
            LOG.warn("konvoy.lock is not recognized as a TOML file; cannot parse")
            return null
        }
        return try {
            parseLockfileFromPsi(psi)
        } catch (e: Exception) {
            LOG.warn("Failed to parse konvoy.lock", e)
            null
        }
    }

    // -- PSI-based parsing --

    /**
     * Build [OpenApiCodegen] only when all required keys are present. An
     * incomplete optional `[codegen.openapi]` section yields null rather than
     * failing the whole manifest parse (which would break IDE sync mid-edit).
     */
    private fun openApiCodegenOf(version: String?, spec: String?, basePackage: String?): OpenApiCodegen? =
        if (version != null && spec != null && basePackage != null) {
            OpenApiCodegen(version, spec, basePackage)
        } else {
            null
        }

    private fun parseManifestFromPsi(file: TomlFile): KonvoyManifest? {
        val tables = file.children.filterIsInstance<TomlTable>()

        val pkgTable = tables.find { it.header.key?.text == "package" }
        val toolchainTable = tables.find { it.header.key?.text == "toolchain" }
        val depsTable = tables.find { it.header.key?.text == "dependencies" }
        val pluginsTable = tables.find { it.header.key?.text == "plugins" }
        val openApiCodegenTable = tables.find {
            it.header.key?.segments?.joinToString(".") { seg -> seg.text } == "codegen.openapi"
        }

        if (pkgTable == null || toolchainTable == null) {
            LOG.warn("konvoy.toml missing required [package] or [toolchain] section")
            return null
        }

        val pkg = KonvoyPackage(
            name = pkgTable.stringValue("name") ?: return null,
            kind = pkgTable.stringValue("kind")?.let { PackageKind.fromString(it) } ?: PackageKind.BIN,
            version = pkgTable.stringValue("version"),
            entrypoint = pkgTable.stringValue("entrypoint") ?: "src/main.kt",
        )

        val toolchain = KonvoyToolchain(
            kotlin = toolchainTable.stringValue("kotlin") ?: return null,
            detekt = toolchainTable.stringValue("detekt"),
        )

        val dependencies = parseDependencySpecs(tables, "dependencies")
        val plugins = parseDependencySpecs(tables, "plugins")
        // An incomplete optional [codegen.openapi] section (e.g. mid-edit) must
        // NOT fail the whole manifest parse — that would abort IDE sync and drop
        // source roots/libraries on every save. Treat it as "no codegen" until
        // all three keys are present; the annotator flags the missing ones.
        val codegen = KonvoyCodegen(
            openapi = openApiCodegenTable?.let {
                openApiCodegenOf(
                    it.stringValue("version"),
                    it.stringValue("spec"),
                    it.stringValue("base_package"),
                )
            },
        )

        return KonvoyManifest(
            `package` = pkg,
            toolchain = toolchain,
            codegen = codegen,
            dependencies = dependencies,
            plugins = plugins,
        )
    }

    /**
     * Parse dotted sub-tables like `[dependencies.my-lib]` or inline tables
     * like `my-lib = { path = "..." }` under a `[dependencies]` table.
     */
    private fun parseDependencySpecs(tables: List<TomlTable>, sectionName: String): Map<String, DependencySpec> {
        val result = mutableMapOf<String, DependencySpec>()

        // Dotted sub-tables: [dependencies.foo] or [plugins.kotlin-serialization]
        for (table in tables) {
            val segments = table.header.key?.segments ?: continue
            if (segments.size == 2 && segments[0].text == sectionName) {
                val depName = segments[1].text
                result[depName] = DependencySpec(
                    path = table.stringValue("path"),
                    version = table.stringValue("version"),
                    maven = table.stringValue("maven"),
                )
            }
        }

        // Inline tables: foo = { path = "..." } under [dependencies]
        val sectionTable = tables.find { it.header.key?.text == sectionName }
        if (sectionTable != null) {
            for (entry in sectionTable.entries) {
                val depName = entry.key.text
                val inlineTable = entry.value as? TomlInlineTable ?: continue
                result[depName] = DependencySpec(
                    path = inlineTable.stringValue("path"),
                    version = inlineTable.stringValue("version"),
                    maven = inlineTable.stringValue("maven"),
                )
            }
        }

        return result
    }

    private fun parseLockfileFromPsi(file: TomlFile): KonvoyLockfile {
        val tables = file.children.filterIsInstance<TomlTable>()
        val arrayTables = file.children.filterIsInstance<TomlArrayTable>()

        val toolchainTable = tables.find { it.header.key?.text == "toolchain" }
        val toolchain = toolchainTable?.let {
            ToolchainLock(
                konancVersion = it.stringValue("konanc_version") ?: "",
                konancTarballSha256 = it.stringValue("konanc_tarball_sha256"),
                jreTarballSha256 = it.stringValue("jre_tarball_sha256"),
                detektVersion = it.stringValue("detekt_version"),
                detektJarSha256 = it.stringValue("detekt_jar_sha256"),
            )
        }

        val deps = mutableListOf<DependencyLock>()
        val plugins = mutableListOf<PluginLock>()
        val codegenTools = mutableMapOf<String, CodegenToolLock>()

        for (table in tables) {
            val segments = table.header.key?.segments ?: continue
            if (segments.size == 2 && segments[0].text == "codegen_tools") {
                val id = segments[1].text
                val version = table.stringValue("version") ?: continue
                val sha256 = table.stringValue("sha256") ?: continue
                codegenTools[id] = CodegenToolLock(version = version, sha256 = sha256)
            }
        }

        for (arrayTable in arrayTables) {
            val headerText = arrayTable.header.key?.text ?: continue
            when (headerText) {
                "dependencies" -> parseDependencyLock(arrayTable)?.let { deps.add(it) }
                "plugins" -> parsePluginLock(arrayTable)?.let { plugins.add(it) }
            }
        }

        return KonvoyLockfile(
            toolchain = toolchain,
            codegenTools = codegenTools,
            dependencies = deps,
            plugins = plugins,
        )
    }

    private fun parseDependencyLock(table: TomlArrayTable): DependencyLock? {
        val name = table.stringValue("name") ?: return null
        val sourceType = table.stringValue("source_type") ?: return null
        val sourceHash = table.stringValue("source_hash") ?: ""

        val source = when (sourceType) {
            "path" -> DepSource.Path(path = table.stringValue("path") ?: "")
            "maven" -> {
                // The engine emits one [dependencies.targets] sub-table directly
                // after each [[dependencies]] entry, so attribute targets to THIS
                // dep by taking the immediately-following sibling table (stopping
                // at the next array-table) rather than merging every targets table.
                val targets = mutableMapOf<String, String>()
                var sibling = table.nextSibling
                while (sibling != null) {
                    if (sibling is TomlArrayTable) break
                    if (sibling is TomlTable) {
                        val segs = sibling.header.key?.segments
                        if (segs != null && segs.size == 2 &&
                            segs[0].text == "dependencies" && segs[1].text == "targets"
                        ) {
                            for (entry in sibling.entries) {
                                val v = (entry.value as? TomlLiteral)?.stringValue()
                                if (v != null) targets[entry.key.text] = v
                            }
                        }
                        break
                    }
                    sibling = sibling.nextSibling
                }
                DepSource.Maven(
                    version = table.stringValue("version") ?: "",
                    maven = table.stringValue("maven") ?: "",
                    targets = targets,
                    classifier = table.stringValue("classifier"),
                )
            }
            else -> return null
        }

        return DependencyLock(name, source, sourceHash)
    }

    private fun parsePluginLock(table: TomlArrayTable): PluginLock? {
        return PluginLock(
            name = table.stringValue("name") ?: return null,
            maven = table.stringValue("maven") ?: return null,
            version = table.stringValue("version") ?: return null,
            sha256 = table.stringValue("sha256") ?: "",
            url = table.stringValue("url") ?: "",
        )
    }

    // -- PSI helpers --

    private fun TomlTable.stringValue(key: String): String? {
        val entry = entries.find { it.key.text == key } ?: return null
        return (entry.value as? TomlLiteral)?.stringValue()
    }

    private fun TomlArrayTable.stringValue(key: String): String? {
        val entry = entries.find { it.key.text == key } ?: return null
        return (entry.value as? TomlLiteral)?.stringValue()
    }

    private fun TomlInlineTable.stringValue(key: String): String? {
        val entry = entries.find { it.key.text == key } ?: return null
        return (entry.value as? TomlLiteral)?.stringValue()
    }

    private fun TomlLiteral.stringValue(): String? {
        // TomlLiteral text includes quotes; strip them
        val text = text ?: return null
        return when {
            text.startsWith("\"\"\"") -> text.removeSurrounding("\"\"\"")
            text.startsWith("\"") -> text.removeSurrounding("\"")
            text.startsWith("'") -> text.removeSurrounding("'")
            else -> text
        }
    }
}
