package com.konvoy.ide.sync

import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.WriteAction
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.module.Module
import com.intellij.openapi.module.ModuleManager
import com.intellij.openapi.module.StdModuleTypes
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.*
import com.intellij.openapi.roots.impl.libraries.LibraryEx
import com.intellij.openapi.roots.libraries.LibraryTablesRegistrar
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VfsUtil
import com.konvoy.ide.config.*
import org.jetbrains.kotlin.cli.common.arguments.K2NativeCompilerArguments
import org.jetbrains.kotlin.config.CompilerSettings
import org.jetbrains.kotlin.idea.base.platforms.KotlinNativeLibraryKind
import org.jetbrains.kotlin.idea.facet.KotlinFacet
import org.jetbrains.kotlin.idea.facet.KotlinFacetType
import org.jetbrains.kotlin.platform.konan.NativePlatforms
import com.intellij.facet.FacetManager
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
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
                } catch (e: Throwable) {
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
        configureLibraries(project, module, manifest, lockfile, basePath)

        try {
            configureKotlinFacet(module, manifest, lockfile)
            LOG.info("Kotlin facet configured successfully")
        } catch (e: Exception) {
            LOG.error("Failed to configure Kotlin facet, language intelligence may be limited", e)
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
        basePath: String,
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
            LOG.warn("Kotlin $kotlinVersion toolchain not installed at $konvoyHome/toolchains/$kotlinVersion")
            NotificationGroupManager.getInstance()
                .getNotificationGroup("Konvoy")
                .createNotification(
                    "Konvoy: Kotlin $kotlinVersion toolchain not found",
                    "The Kotlin version specified in konvoy.toml requires toolchain $kotlinVersion, " +
                        "but it is not installed at <code>~/.konvoy/toolchains/$kotlinVersion/</code>. " +
                        "Code intelligence will be limited until the toolchain is available.<br/><br/>" +
                        "Run <code>konvoy toolchain install</code> to download it.",
                    NotificationType.WARNING,
                )
                .notify(project)
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
                        val depPath = File(basePath, source.path).canonicalPath
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
     * Find the Kotlin/Native stdlib klib for the exact requested version.
     * Returns the path or null if the toolchain isn't installed.
     */
    private fun findStdlibKlib(konvoyHome: String, kotlinVersion: String): String? {
        val path = "$konvoyHome/toolchains/$kotlinVersion/klib/common/stdlib"
        if (File(path, "default/manifest").exists()) return path
        return null
    }

    private fun addKlibLibrary(
        tableModel: com.intellij.openapi.roots.libraries.LibraryTable.ModifiableModel,
        moduleModel: ModifiableRootModel,
        name: String,
        klibPath: String,
    ) {
        val lib = tableModel.createLibrary(name)
        val libModel = lib.modifiableModel
        val file = File(klibPath)

        // Refresh VFS so K2 analyzer can discover the klib files.
        // Without this, getRootProvider().getFiles(CLASSES) returns empty
        // and KaLibraryModuleImpl.resolvedKotlinLibraries finds nothing.
        val vFile = LocalFileSystem.getInstance().refreshAndFindFileByPath(file.absolutePath)
        if (vFile != null) {
            VfsUtil.markDirtyAndRefresh(false, true, true, vFile)
        }

        val url = if (file.isDirectory) {
            com.intellij.openapi.vfs.VfsUtilCore.pathToUrl(file.absolutePath)
        } else {
            VfsUtil.getUrlForLibraryRoot(file)
        }
        LOG.info("Adding library '$name' with URL: $url (vfs=${vFile != null})")
        libModel.addRoot(url, OrderRootType.CLASSES)

        // Mark as Kotlin/Native library so the Kotlin plugin recognizes it
        if (libModel is LibraryEx.ModifiableModelEx) {
            libModel.kind = KotlinNativeLibraryKind
            LOG.info("Set library kind to KotlinNativeLibraryKind for '$name'")
        }

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

        // Set Kotlin/Native compiler arguments — this is what the targetPlatform getter
        // uses to derive the platform in K2 mode (setting targetPlatform directly is ignored)
        val nativeArgs = K2NativeCompilerArguments()
        val kotlinVersion = manifest.toolchain.kotlin
        nativeArgs.languageVersion = kotlinVersion
        nativeArgs.apiVersion = kotlinVersion

        // Configure compiler plugins
        if (lockfile != null && lockfile.plugins.isNotEmpty()) {
            val konvoyHome = System.getProperty("user.home") + "/.konvoy"
            val pluginPaths = lockfile.plugins.mapNotNull { plugin ->
                val (groupId, artifactId) = plugin.maven.split(":", limit = 2).takeIf { it.size == 2 }
                    ?: return@mapNotNull null
                val groupPath = groupId.replace('.', '/')
                val path = "$konvoyHome/cache/maven/$groupPath/$artifactId/${plugin.version}/$artifactId-${plugin.version}.jar"
                if (File(path).exists()) path else {
                    val altPath = "$konvoyHome/tools/plugins/${plugin.name}/${plugin.version}/$artifactId-${plugin.version}.jar"
                    if (File(altPath).exists()) altPath else null
                }
            }
            nativeArgs.pluginClasspaths = pluginPaths.toTypedArray()
        }

        settings.compilerArguments = nativeArgs

        // Set compilerSettings — this is what hasKotlinPluginEnabled() checks
        // for non-JPS build systems (which we report via KonvoyBuildSystemTypeDetector)
        if (settings.compilerSettings == null) {
            settings.compilerSettings = CompilerSettings()
        }

        // Set target platform explicitly (backed by compiler arguments above)
        val konanTarget = KonvoyTargets.hostTarget()
        settings.targetPlatform = NativePlatforms.nativePlatformBySingleTarget(konanTarget)

        // Set language and API version from toolchain
        settings.languageLevel = org.jetbrains.kotlin.config.LanguageVersion.fromVersionString(kotlinVersion)
        settings.apiLevel = org.jetbrains.kotlin.config.LanguageVersion.fromVersionString(kotlinVersion)

        LOG.info("Configured Kotlin facet: platform=Native, kotlin=$kotlinVersion, compilerSettings=present")
    }

}
