package com.konvoy.ide.sdk

import com.intellij.openapi.projectRoots.*
import com.intellij.openapi.roots.OrderRootType
import com.intellij.openapi.vfs.VfsUtil
import org.jdom.Element
import java.io.File

/**
 * Custom SDK type for Konvoy-managed Kotlin/Native toolchains.
 * Discovers toolchains installed at ~/.konvoy/toolchains/<version>/.
 */
class KonvoySdkType : SdkType("KonvoyToolchain") {

    override fun getPresentableName(): String = "Konvoy Toolchain"

    override fun suggestHomePath(): String? {
        val toolchainsDir = toolchainsBaseDir()
        return toolchainsDir.listFiles()
            ?.filter { it.isDirectory }
            ?.maxByOrNull { it.name } // latest version
            ?.absolutePath
    }

    override fun suggestHomePaths(): Collection<String> {
        val toolchainsDir = toolchainsBaseDir()
        if (!toolchainsDir.isDirectory) return emptyList()
        return toolchainsDir.listFiles()
            ?.filter { it.isDirectory && hasKonanc(it) }
            ?.map { it.absolutePath }
            ?: emptyList()
    }

    override fun isValidSdkHome(path: String): Boolean = hasKonanc(File(path))

    override fun suggestSdkName(currentSdkName: String?, sdkHome: String): String {
        val version = getVersionString(sdkHome) ?: File(sdkHome).name
        return "Konvoy $version"
    }

    override fun getVersionString(sdkHome: String): String? {
        // The directory name under ~/.konvoy/toolchains/ is the Kotlin version
        return File(sdkHome).name
    }

    override fun createAdditionalDataConfigurable(
        sdkModel: SdkModel,
        sdkModificator: SdkModificator,
    ): AdditionalDataConfigurable? = null

    override fun saveAdditionalData(additionalData: SdkAdditionalData, additional: Element) {}

    override fun setupSdkPaths(sdk: Sdk) {
        val modificator = sdk.sdkModificator
        val home = sdk.homePath ?: return

        // Add konanc stdlib klib as a class root for resolution
        val stdlibDir = File(home, "klib/common/stdlib")
        if (stdlibDir.exists()) {
            val url = VfsUtil.getUrlForLibraryRoot(stdlibDir)
            modificator.addRoot(url, OrderRootType.CLASSES)
        }

        modificator.commitChanges()
    }

    private fun hasKonanc(dir: File): Boolean {
        return File(dir, "bin/konanc").exists() || File(dir, "bin/konanc.bat").exists()
    }

    companion object {
        fun toolchainsBaseDir(): File =
            File(System.getProperty("user.home"), ".konvoy/toolchains")

        fun getInstance(): KonvoySdkType =
            findInstance(KonvoySdkType::class.java)
    }
}
