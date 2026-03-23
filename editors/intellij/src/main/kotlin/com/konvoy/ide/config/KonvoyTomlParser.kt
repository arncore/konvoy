package com.konvoy.ide.config

import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.vfs.VirtualFile
import org.toml.lang.psi.*
import com.intellij.psi.PsiManager
import com.intellij.openapi.project.Project

/**
 * Parses `konvoy.toml` and `konvoy.lock` files using the TOML PSI.
 *
 * Falls back to manual line parsing when the TOML plugin is unavailable,
 * but the PSI path is preferred for accuracy and IDE integration.
 */
object KonvoyTomlParser {
    private val LOG = Logger.getInstance(KonvoyTomlParser::class.java)

    fun parseManifest(project: Project, file: VirtualFile): KonvoyManifest? {
        return try {
            val psi = PsiManager.getInstance(project).findFile(file) as? TomlFile ?: return null
            parseManifestFromPsi(psi)
        } catch (e: Exception) {
            LOG.warn("Failed to parse konvoy.toml via PSI, falling back to text", e)
            parseManifestFromText(file.contentsToByteArray().decodeToString())
        }
    }

    fun parseLockfile(project: Project, file: VirtualFile): KonvoyLockfile? {
        return try {
            val psi = PsiManager.getInstance(project).findFile(file) as? TomlFile ?: return null
            parseLockfileFromPsi(psi)
        } catch (e: Exception) {
            LOG.warn("Failed to parse konvoy.lock via PSI, falling back to text", e)
            parseLockfileFromText(file.contentsToByteArray().decodeToString())
        }
    }

    // -- PSI-based parsing --

    private fun parseManifestFromPsi(file: TomlFile): KonvoyManifest? {
        val tables = file.children.filterIsInstance<TomlTable>()

        val pkgTable = tables.find { it.header.key?.text == "package" }
        val toolchainTable = tables.find { it.header.key?.text == "toolchain" }
        val depsTable = tables.find { it.header.key?.text == "dependencies" }
        val pluginsTable = tables.find { it.header.key?.text == "plugins" }

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

        return KonvoyManifest(pkg, toolchain, dependencies, plugins)
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

        for (arrayTable in arrayTables) {
            val headerText = arrayTable.header.key?.text ?: continue
            when (headerText) {
                "dependencies" -> parseDependencyLock(arrayTable, tables)?.let { deps.add(it) }
                "plugins" -> parsePluginLock(arrayTable)?.let { plugins.add(it) }
            }
        }

        return KonvoyLockfile(toolchain, deps, plugins)
    }

    private fun parseDependencyLock(table: TomlArrayTable, allTables: List<TomlTable>): DependencyLock? {
        val name = table.stringValue("name") ?: return null
        val sourceType = table.stringValue("source_type") ?: return null
        val sourceHash = table.stringValue("source_hash") ?: ""

        val source = when (sourceType) {
            "path" -> DepSource.Path(path = table.stringValue("path") ?: "")
            "maven" -> {
                // Targets may be in a sub-table [dependencies.targets]
                val targets = mutableMapOf<String, String>()
                for (t in allTables) {
                    val segs = t.header.key?.segments ?: continue
                    if (segs.size == 2 && segs[0].text == "dependencies" && segs[1].text == "targets") {
                        for (entry in t.entries) {
                            val v = (entry.value as? TomlLiteral)?.stringValue()
                            if (v != null) targets[entry.key.text] = v
                        }
                    }
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

    // -- Text-based fallback parsing --

    fun parseManifestFromText(content: String): KonvoyManifest? {
        val sections = parseTomlSections(content)

        val pkgSection = sections["package"] ?: return null
        val toolchainSection = sections["toolchain"] ?: return null

        val pkg = KonvoyPackage(
            name = pkgSection["name"] ?: return null,
            kind = pkgSection["kind"]?.let { PackageKind.fromString(it) } ?: PackageKind.BIN,
            version = pkgSection["version"],
            entrypoint = pkgSection["entrypoint"] ?: "src/main.kt",
        )

        val toolchain = KonvoyToolchain(
            kotlin = toolchainSection["kotlin"] ?: return null,
            detekt = toolchainSection["detekt"],
        )

        val deps = mutableMapOf<String, DependencySpec>()
        for ((key, values) in sections) {
            if (key.startsWith("dependencies.")) {
                val depName = key.removePrefix("dependencies.")
                deps[depName] = DependencySpec(
                    path = values["path"],
                    version = values["version"],
                    maven = values["maven"],
                )
            }
        }
        // Inline deps under [dependencies] need more complex parsing;
        // for now we handle sub-table style which is the common case

        return KonvoyManifest(pkg, toolchain, deps)
    }

    fun parseLockfileFromText(content: String): KonvoyLockfile {
        val sections = parseTomlSections(content)

        val toolchainSection = sections["toolchain"]
        val toolchain = toolchainSection?.let {
            ToolchainLock(
                konancVersion = it["konanc_version"] ?: "",
                konancTarballSha256 = it["konanc_tarball_sha256"],
                jreTarballSha256 = it["jre_tarball_sha256"],
                detektVersion = it["detekt_version"],
                detektJarSha256 = it["detekt_jar_sha256"],
            )
        }

        // Text-based array-of-tables parsing is limited; prefer PSI path
        return KonvoyLockfile(toolchain)
    }

    /**
     * Minimal TOML section parser. Handles `[section]` headers and `key = "value"` entries.
     * Does not handle inline tables or array-of-tables — use PSI parsing for those.
     */
    private fun parseTomlSections(content: String): Map<String, Map<String, String>> {
        val sections = mutableMapOf<String, MutableMap<String, String>>()
        var currentSection = ""

        for (line in content.lines()) {
            val trimmed = line.trim()
            if (trimmed.startsWith('#') || trimmed.isEmpty()) continue

            val sectionMatch = Regex("""\[([^\[\]]+)]""").matchEntire(trimmed)
            if (sectionMatch != null) {
                currentSection = sectionMatch.groupValues[1].trim()
                sections.getOrPut(currentSection) { mutableMapOf() }
                continue
            }

            val kvMatch = Regex("""(\S+)\s*=\s*"([^"]*)"""").find(trimmed)
            if (kvMatch != null && currentSection.isNotEmpty()) {
                sections.getOrPut(currentSection) { mutableMapOf() }[kvMatch.groupValues[1]] = kvMatch.groupValues[2]
            }
        }
        return sections
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
