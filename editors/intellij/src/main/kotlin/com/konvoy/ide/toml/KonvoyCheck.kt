package com.konvoy.ide.toml

import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.openapi.diagnostic.Logger
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * A single `konvoy.toml` diagnostic emitted by `konvoy check --format json`.
 *
 * The plugin is a thin client: konvoy is the single source of validation truth, and
 * the plugin only renders what it reports — it never re-implements any rules.
 * [keyPath] is the dotted TOML path the diagnostic concerns (e.g. `codegen.openapi`
 * or `package.name`); [line]/[column] (1-based) are present for TOML syntax errors.
 */
data class KonvoyCheckDiagnostic(
    val severity: String,
    val message: String,
    val keyPath: String?,
    val line: Int?,
    val column: Int?,
)

/**
 * Runs `konvoy check --format json` and parses its diagnostics. Validation lives in
 * the konvoy backend; this object only invokes it and reads the structured output.
 */
object KonvoyCheck {

    private val LOG = Logger.getInstance(KonvoyCheck::class.java)

    /** Run `konvoy check --format json` in [projectDir] and return its diagnostics. */
    fun run(projectDir: File): List<KonvoyCheckDiagnostic> {
        return try {
            val process = ProcessBuilder("konvoy", "check", "--format", "json")
                .directory(projectDir)
                .start()
            // `--format json` prints the array on stdout and always exits 0, so stderr
            // is kept separate and ignored.
            val stdout = process.inputStream.bufferedReader().readText()
            if (!process.waitFor(10, TimeUnit.SECONDS)) {
                process.destroyForcibly()
                LOG.warn("konvoy check timed out")
                return emptyList()
            }
            parse(stdout)
        } catch (e: Exception) {
            // konvoy not on PATH, or any other failure — surface no diagnostics rather
            // than breaking editing.
            LOG.info("konvoy check unavailable: ${e.message}")
            emptyList()
        }
    }

    /** Parse the JSON array printed by `konvoy check --format json`. */
    fun parse(json: String): List<KonvoyCheckDiagnostic> {
        val array = try {
            JsonParser.parseString(json).asJsonArray
        } catch (e: Exception) {
            return emptyList()
        }
        return array.mapNotNull { element ->
            if (!element.isJsonObject) return@mapNotNull null
            val obj = element.asJsonObject
            val message = obj.string("message") ?: return@mapNotNull null
            KonvoyCheckDiagnostic(
                severity = obj.string("severity") ?: "error",
                message = message,
                keyPath = obj.string("key_path"),
                line = obj.int("line"),
                column = obj.int("column"),
            )
        }
    }

    private fun JsonObject.string(key: String): String? =
        get(key)?.takeIf { it.isJsonPrimitive }?.asString

    private fun JsonObject.int(key: String): Int? =
        get(key)?.takeIf { it.isJsonPrimitive }?.asInt
}
