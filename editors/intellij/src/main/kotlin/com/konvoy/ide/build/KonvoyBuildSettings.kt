package com.konvoy.ide.build

import com.intellij.openapi.components.*
import com.intellij.openapi.project.Project

/**
 * Persistent project-level settings for the Konvoy build system.
 */
@Service(Service.Level.PROJECT)
@State(
    name = "KonvoyBuildSettings",
    storages = [Storage("konvoy.xml")],
)
class KonvoyBuildSettings : PersistentStateComponent<KonvoyBuildSettings.State> {

    data class State(
        var buildOnSave: Boolean = false,
    )

    private var myState = State()

    override fun getState(): State = myState

    override fun loadState(state: State) {
        myState = state
    }

    companion object {
        fun getInstance(project: Project): KonvoyBuildSettings =
            project.getService(KonvoyBuildSettings::class.java)
    }
}
