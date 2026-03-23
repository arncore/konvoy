package com.konvoy.ide.sync

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.WriteAction
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.module.Module
import com.intellij.openapi.module.ModuleManager
import com.intellij.openapi.module.StdModuleTypes
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.*
import com.intellij.openapi.roots.libraries.LibraryTablesRegistrar
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VfsUtil
import com.konvoy.ide.config.*
import org.jetbrains.kotlin.config.KotlinFacetSettingsProvider
import org.jetbrains.kotlin.idea.facet.KotlinFacet
import org.jetbrains.kotlin.idea.facet.KotlinFacetType
import org.jetbrains.kotlin.platform.konan.NativePlatforms
import com.intellij.facet.FacetManager
import java.io.File

/**
 * Updates the IntelliJ workspace model to reflect the current Konvoy project state.
 *
 * This is the bridge between konvoy.toml/konvoy.lock and IntelliJ's project model.
 * It configures:
 * - Module with source roots (src/, excluding src/test/) and test roots (src/test/)
 * - Library dependencies (klibs from ~/.konvoy/cache/maven/)
 * - Kotlin facet with Native target platform
 * - Compiler plugin configuration
 */
object KonvoyWorkspaceModelUpdater {
    private val LOG = Logger.getInstance(KonvoyWorkspaceModelUpdater::class.java)

    fun updateProjectModel(project: Project, manifest: KonvoyManifest, lockfile: KonvoyLockfile?) {
        LOG.info("Scheduling workspace model update for '${manifest.`package`.name}'")
        ApplicationManager.getApplication().invokeLater {
            if (project.isDisposed) return@invokeLater
            WriteAction.run<Throwable> {
                try {
                    doUpdate(project, manifest, lockfile)
                } catch (e: Exception) {
                    LOG.error("Failed to update workspace model", e)
                }
            }
        }
    }

    private fun doUpdate(project: Project, manifest: KonvoyManifest, lockfile: KonvoyLockfile?) {
        val basePath = project.basePath ?: return
        val moduleName = manifest.`package`.name

        LOG.info("Updating workspace model for '$moduleName' (kotlin ${manifest.toolchain.kotlin})")

        val module = getOrCreateModule(project, moduleName, basePath)
        configureSourceRoots(module, basePath)
        configureLibraries(project, module, manifest, lockfile)

        try {
            configureKotlinFacet(module, manifest, lockfile)
        } catch (e: Exception) {
            LOG.warn("Failed to configure Kotlin facet, language intelligence may be limited", e)
        }

        LOG.info("Workspace model updated for module '$moduleName'")
    }

    private fun getOrCreateModule(project: Project, name: String, basePath: String): Module {
        val moduleManager = ModuleManager.getInstance(project)

        // Reuse existing module if present
        moduleManager.findModuleByName(name)?.let { return it }

        // Create new module
        val imlPath = "$basePath/.konvoy/$name.iml"
        val model = moduleManager.getModifiableModel()
        val module = model.newModule(imlPath, StdModuleTypes.JAVA.id)
        model.commit()
        return module
    }

    private fun configureSourceRoots(module: Module, basePath: String) {
        val model = ModuleRootManager.getInstance(module).modifiableModel

        // Clear existing content roots to avoid duplicates on re-sync
        model.contentEntries.forEach { model.removeContentEntry(it) }

        val projectDir = LocalFileSystem.getInstance().refreshAndFindFileByPath(basePath) ?: run {
            model.dispose()
            return
        }

        val contentEntry = model.addContentEntry(projectDir)

        // src/ as source root (excluding src/test/)
        val srcDir = projectDir.findChild("src")
        if (srcDir != null) {
            contentEntry.addSourceFolder(srcDir, false)
        }

        // src/test/ as test source root
        val testDir = srcDir?.findChild("test")
        if (testDir != null) {
            contentEntry.addSourceFolder(testDir, true)
        }

        // Exclude build output directory
        val buildDir = projectDir.findChild(".konvoy")
        if (buildDir != null) {
            contentEntry.addExcludeFolder(buildDir)
        }

        model.commit()
    }

    private fun configureLibraries(
        project: Project,
        module: Module,
        manifest: KonvoyManifest,
        lockfile: KonvoyLockfile?,
    ) {
        val libraryTable = LibraryTablesRegistrar.getInstance().getLibraryTable(project)
        val moduleModel = ModuleRootManager.getInstance(module).modifiableModel
        val hostTarget = KonvoyTargets.hostTargetName()
        val mavenSuffix = KonvoyTargets.toMavenSuffix(hostTarget)
        val konvoyHome = System.getProperty("user.home") + "/.konvoy"

        // Remove old Konvoy libraries to avoid stale entries
        val existingLibs = libraryTable.libraries.filter { it.name?.startsWith("konvoy:") == true }
        if (existingLibs.isNotEmpty()) {
            val tableModel = libraryTable.modifiableModel
            existingLibs.forEach { tableModel.removeLibrary(it) }
            tableModel.commit()
        }

        // Remove old library dependencies from module
        moduleModel.orderEntries
            .filterIsInstance<LibraryOrderEntry>()
            .filter { it.libraryName?.startsWith("konvoy:") == true }
            .forEach { moduleModel.removeOrderEntry(it) }

        val tableModel = libraryTable.modifiableModel

        // Add Kotlin/Native stdlib from the managed toolchain
        val kotlinVersion = manifest.toolchain.kotlin
        val stdlibPath = findStdlibKlib(konvoyHome, kotlinVersion)
        if (stdlibPath != null) {
            addKlibLibrary(tableModel, moduleModel, "konvoy:kotlin-stdlib", stdlibPath)
            LOG.info("Added Kotlin/Native stdlib from $stdlibPath")
        } else {
            LOG.warn("Kotlin/Native stdlib not found for version $kotlinVersion in $konvoyHome/toolchains/")
        }

        // Add Maven dependencies as libraries
        if (lockfile != null) {
            for (dep in lockfile.dependencies) {
                val source = dep.source
                when (source) {
                    is DepSource.Maven -> {
                        val klibPath = resolveKlibPath(konvoyHome, source, mavenSuffix)
                        if (klibPath != null) {
                            addKlibLibrary(tableModel, moduleModel, "konvoy:${dep.name}", klibPath)
                        }
                    }
                    is DepSource.Path -> {
                        val depPath = File(project.basePath!!, source.path).canonicalPath
                        addPathDependency(tableModel, moduleModel, "konvoy:${dep.name}", depPath)
                    }
                }
            }
        }

        // Also add path dependencies from manifest (may not be in lockfile yet)
        for ((name, spec) in manifest.dependencies) {
            if (spec.isPath && spec.path != null) {
                val depPath = File(project.basePath!!, spec.path).canonicalPath
                val libName = "konvoy:$name"
                if (tableModel.getLibraryByName(libName) == null) {
                    addPathDependency(tableModel, moduleModel, libName, depPath)
                }
            }
        }

        tableModel.commit()
        moduleModel.commit()
    }

    private fun resolveKlibPath(konvoyHome: String, source: DepSource.Maven, mavenSuffix: String): String? {
        val (groupId, artifactId) = source.maven.split(":", limit = 2).takeIf { it.size == 2 } ?: return null
        val groupPath = groupId.replace('.', '/')
        val version = source.version

        val classifier = source.classifier
        val fileName = if (classifier != null) {
            "$artifactId-$mavenSuffix-$version-$classifier.klib"
        } else {
            "$artifactId-$mavenSuffix-$version.klib"
        }

        val path = "$konvoyHome/cache/maven/$groupPath/$artifactId/$version/$fileName"
        return if (File(path).exists()) path else null
    }

    /**
     * Find the Kotlin/Native stdlib klib in the managed toolchain.
     * Tries the exact version first, then falls back to the closest available version.
     */
    private fun findStdlibKlib(konvoyHome: String, kotlinVersion: String): String? {
        val exactPath = "$konvoyHome/toolchains/$kotlinVersion/klib/common/stdlib"
        if (File(exactPath).isDirectory) return exactPath

        // Fallback: find any installed toolchain with a stdlib
        val toolchainsDir = File("$konvoyHome/toolchains")
        if (!toolchainsDir.isDirectory) return null
        return toolchainsDir.listFiles()
            ?.filter { it.isDirectory && File(it, "klib/common/stdlib").isDirectory }
            ?.maxByOrNull { it.name } // prefer latest version
            ?.let { "${it.absolutePath}/klib/common/stdlib" }
    }

    private fun addKlibLibrary(
        tableModel: com.intellij.openapi.roots.libraries.LibraryTable.ModifiableModel,
        moduleModel: ModifiableRootModel,
        name: String,
        klibPath: String,
    ) {
        val lib = tableModel.createLibrary(name)
        val libModel = lib.modifiableModel
        val url = VfsUtil.getUrlForLibraryRoot(File(klibPath))
        libModel.addRoot(url, OrderRootType.CLASSES)
        libModel.commit()
        moduleModel.addLibraryEntry(lib)
    }

    private fun addPathDependency(
        tableModel: com.intellij.openapi.roots.libraries.LibraryTable.ModifiableModel,
        moduleModel: ModifiableRootModel,
        name: String,
        depPath: String,
    ) {
        val srcDir = File(depPath, "src")
        if (!srcDir.exists()) return

        val lib = tableModel.createLibrary(name)
        val libModel = lib.modifiableModel
        val url = VfsUtil.getUrlForLibraryRoot(srcDir)
        libModel.addRoot(url, OrderRootType.SOURCES)
        libModel.commit()
        moduleModel.addLibraryEntry(lib)
    }

    private fun configureKotlinFacet(module: Module, manifest: KonvoyManifest, lockfile: KonvoyLockfile?) {
        val facetManager = FacetManager.getInstance(module)
        val existingFacet = facetManager.getFacetByType(KotlinFacetType.TYPE_ID)

        val facet = existingFacet ?: run {
            val model = facetManager.createModifiableModel()
            val newFacet = facetManager.createFacet(
                KotlinFacetType.INSTANCE,
                KotlinFacetType.NAME,
                null,
            )
            model.addFacet(newFacet)
            model.commit()
            newFacet
        }

        val settings = facet.configuration.settings

        // Set Kotlin/Native target platform
        val konanTarget = KonvoyTargets.hostTarget()
        settings.targetPlatform = NativePlatforms.nativePlatformBySingleTarget(konanTarget)

        // Set language and API version from toolchain
        val kotlinVersion = manifest.toolchain.kotlin
        settings.languageLevel = org.jetbrains.kotlin.config.LanguageVersion.fromVersionString(kotlinVersion)
        settings.apiLevel = org.jetbrains.kotlin.config.LanguageVersion.fromVersionString(kotlinVersion)

        // Configure compiler plugins
        if (lockfile != null && lockfile.plugins.isNotEmpty()) {
            val konvoyHome = System.getProperty("user.home") + "/.konvoy"
            val pluginPaths = lockfile.plugins.mapNotNull { plugin ->
                val (groupId, artifactId) = plugin.maven.split(":", limit = 2).takeIf { it.size == 2 }
                    ?: return@mapNotNull null
                val groupPath = groupId.replace('.', '/')
                val path = "$konvoyHome/cache/maven/$groupPath/$artifactId/${plugin.version}/$artifactId-${plugin.version}.jar"
                if (File(path).exists()) path else {
                    // Plugins may also be at the download URL path structure
                    val altPath = "$konvoyHome/tools/plugins/${plugin.name}/${plugin.version}/$artifactId-${plugin.version}.jar"
                    if (File(altPath).exists()) altPath else null
                }
            }

            val args = settings.compilerArguments
            if (args != null) {
                args.pluginClasspaths = pluginPaths.toTypedArray()
            }
        }
    }

}
