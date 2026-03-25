package com.konvoy.ide.toml

import com.intellij.codeInsight.daemon.LineMarkerInfo
import com.intellij.codeInsight.daemon.LineMarkerProvider
import com.intellij.icons.AllIcons
import com.intellij.openapi.editor.markup.GutterIconRenderer
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.psi.PsiElement
import org.toml.lang.psi.TomlTable

/**
 * Adds a gutter icon on dependency/plugin sub-table headers in `konvoy.toml`
 * that navigates to the corresponding entry in `konvoy.lock`.
 *
 * e.g., clicking the icon on `[dependencies.serialization-core]` jumps to
 * the `[[dependencies]]` entry with `name = "serialization-core"` in konvoy.lock.
 */
class KonvoyTomlLineMarkerProvider : LineMarkerProvider {

    override fun getLineMarkerInfo(element: PsiElement): LineMarkerInfo<*>? {
        if (element.containingFile?.name != "konvoy.toml") return null

        // Walk up to find the TomlTable — we anchor on leaf elements
        val table = findParentTable(element) ?: return null
        val headerKey = table.header.key ?: return null
        val segments = headerKey.segments
        if (segments.size != 2) return null

        // Only trigger on the leaf node of the first segment to avoid duplicates
        val firstSegment = segments[0]
        if (element != firstSegment.firstChild && element != firstSegment) return null

        val section = segments[0].text
        val name = segments[1].text

        if (section != "dependencies" && section != "plugins") return null

        // Verify the element text matches what we expect
        if (element.text != section && element.text != firstSegment.firstChild?.text) return null

        return LineMarkerInfo(
            element,
            element.textRange,
            AllIcons.Gutter.ImplementedMethod,
            { "Navigate to $name in konvoy.lock" },
            { _, _ -> navigateToLockEntry(element, section, name) },
            GutterIconRenderer.Alignment.LEFT,
            { "Navigate to konvoy.lock" },
        )
    }

    private fun findParentTable(element: PsiElement): TomlTable? {
        var current = element.parent
        // Walk up at most 3 levels (leaf → segment → key → header → table)
        repeat(4) {
            if (current is TomlTable) return current as TomlTable
            current = current?.parent ?: return null
        }
        return null
    }

    private fun navigateToLockEntry(element: PsiElement, section: String, name: String) {
        val basePath = element.project.basePath ?: return
        val lockFile = LocalFileSystem.getInstance().findFileByPath("$basePath/konvoy.lock") ?: return

        val content = lockFile.contentsToByteArray().decodeToString()
        val lines = content.lines()

        var inSection = false
        for ((index, line) in lines.withIndex()) {
            val trimmed = line.trim()
            if (trimmed == "[[$section]]") {
                inSection = true
                continue
            }
            if (trimmed.startsWith("[[") && trimmed.endsWith("]]")) {
                inSection = false
                continue
            }
            if (inSection && trimmed.startsWith("name") && trimmed.contains("\"$name\"")) {
                val descriptor = OpenFileDescriptor(element.project, lockFile, index, 0)
                FileEditorManager.getInstance(element.project).openTextEditor(descriptor, true)
                return
            }
        }

        // Fallback: just open the lock file
        val descriptor = OpenFileDescriptor(element.project, lockFile, 0, 0)
        FileEditorManager.getInstance(element.project).openTextEditor(descriptor, true)
    }
}
