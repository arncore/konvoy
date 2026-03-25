package com.konvoy.ide.run

import com.intellij.execution.lineMarker.ExecutorAction
import com.intellij.execution.lineMarker.RunLineMarkerContributor
import com.intellij.icons.AllIcons
import com.intellij.psi.PsiElement
import com.konvoy.ide.sync.KonvoyProjectService
import org.jetbrains.kotlin.psi.KtNamedFunction

/**
 * Adds a green play icon in the gutter next to `fun main()` declarations
 * in Konvoy bin projects. Clicking it runs `konvoy run`.
 */
class KonvoyRunLineMarkerContributor : RunLineMarkerContributor() {

    override fun getInfo(element: PsiElement): Info? {
        // RunLineMarkerContributor requires we match on leaf elements (the identifier)
        val parent = element.parent
        if (parent !is KtNamedFunction) return null
        if (parent.nameIdentifier != element) return null

        val service = KonvoyProjectService.getInstance(element.project)
        if (!service.isKonvoyProject) return null

        if (KonvoyPsiUtils.findMainFunction(element, element.project) == null) return null

        val actions = ExecutorAction.getActions(0)
        return Info(
            AllIcons.RunConfigurations.TestState.Run,
            actions,
            { "Run with Konvoy" },
        )
    }
}
