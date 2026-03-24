package com.konvoy.ide.sync

import com.intellij.openapi.module.Module
import org.jetbrains.kotlin.idea.configuration.BuildSystemType
import org.jetbrains.kotlin.idea.configuration.BuildSystemTypeDetector

/**
 * Tells the Kotlin plugin that Konvoy modules are NOT JPS-managed.
 *
 * For non-JPS modules, `hasKotlinPluginEnabled()` checks whether the
 * KotlinFacet has non-null `compilerSettings` instead of scanning the
 * classpath for the Kotlin runtime JAR. This lets us control the
 * "Kotlin is not configured" banner via facet configuration alone.
 */
class KonvoyBuildSystemTypeDetector : BuildSystemTypeDetector {
    override fun detectBuildSystemType(module: Module): BuildSystemType? {
        val project = module.project
        val service = KonvoyProjectService.getInstance(project)
        if (service.isKonvoyProject) {
            // Use Gradle as a stand-in — the enum doesn't have a Konvoy variant,
            // and any non-JPS value triggers the facet-based check path.
            return BuildSystemType.Gradle
        }
        return null
    }
}
