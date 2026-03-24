package com.konvoy.ide.sync

import com.intellij.openapi.module.Module
import com.intellij.openapi.project.Project
import org.jetbrains.kotlin.config.ApiVersion
import org.jetbrains.kotlin.config.LanguageFeature
import org.jetbrains.kotlin.idea.base.projectStructure.ModuleSourceRootGroup
import org.jetbrains.kotlin.idea.configuration.ConfigureKotlinStatus
import org.jetbrains.kotlin.idea.configuration.KotlinProjectConfigurator
import org.jetbrains.kotlin.platform.TargetPlatform
import org.jetbrains.kotlin.platform.konan.NativePlatforms

/**
 * Tells the Kotlin plugin that Konvoy-managed modules are already configured.
 *
 * This suppresses the "Kotlin is not configured" editor banner by reporting
 * [ConfigureKotlinStatus.CONFIGURED] for modules that belong to a Konvoy project.
 */
class KonvoyKotlinProjectConfigurator : KotlinProjectConfigurator {

    override val presentableText: String = "Konvoy"

    override val name: String = "konvoy"

    override val targetPlatform: TargetPlatform = NativePlatforms.unspecifiedNativePlatform

    override fun isApplicable(module: Module): Boolean {
        return KonvoyProjectService.getInstance(module.project).isKonvoyProject
    }

    override fun getStatus(moduleSourceRootGroup: ModuleSourceRootGroup): ConfigureKotlinStatus {
        val service = KonvoyProjectService.getInstance(moduleSourceRootGroup.baseModule.project)
        return if (service.isKonvoyProject) {
            ConfigureKotlinStatus.CONFIGURED
        } else {
            ConfigureKotlinStatus.NON_APPLICABLE
        }
    }

    override fun configure(project: Project, modules: Collection<Module>) {
        KonvoyProjectService.getInstance(project).sync()
    }

    override fun updateLanguageVersion(
        module: Module,
        languageVersion: String?,
        apiVersion: String?,
        requiredStdlibVersion: ApiVersion,
        forTests: Boolean,
    ) {
        // Language version is managed by konvoy.toml [toolchain] section
    }

    override fun changeGeneralFeatureConfiguration(
        module: Module,
        feature: LanguageFeature,
        state: LanguageFeature.State,
        forTests: Boolean,
    ) {
        // Feature configuration is managed by Konvoy
    }
}
