package com.konvoy.ide.run

import com.intellij.execution.lineMarker.ExecutorAction
import com.intellij.execution.lineMarker.RunLineMarkerContributor
import com.intellij.icons.AllIcons
import com.intellij.psi.PsiElement
import com.konvoy.ide.sync.KonvoyProjectService
import org.jetbrains.kotlin.psi.KtNamedFunction

/**
 * Adds a green play icon in the gutter next to `@Test` functions in `src/test/`.
 * Clicking it runs `konvoy test --filter=<name>`.
 */
class KonvoyTestLineMarkerContributor : RunLineMarkerContributor() {

    override fun getInfo(element: PsiElement): Info? {
        val parent = element.parent
        if (parent !is KtNamedFunction) return null
        if (parent.nameIdentifier != element) return null

        val service = KonvoyProjectService.getInstance(element.project)
        if (!service.isKonvoyProject) return null

        val testFunction = KonvoyPsiUtils.findTestFunction(element, element.project) ?: return null
        val testName = testFunction.name ?: return null

        val actions = ExecutorAction.getActions(0)
        return Info(
            AllIcons.RunConfigurations.TestState.Run,
            actions,
            { "Test $testName with Konvoy" },
        )
    }
}
