package com.konvoy.ide.run

/**
 * Formats editable run-configuration arguments that IntelliJ later parses with
 * `ParametersList.parse` before starting the `konvoy` process.
 */
object KonvoyCommandLineArgs {

    fun testFilterExtraArgs(filter: String): String {
        require(filter.isNotEmpty()) { "test filter must not be empty" }
        return "--filter ${quoteIfNeeded(filter)}"
    }

    private fun quoteIfNeeded(value: String): String {
        if (value.none { it.isWhitespace() || it == '"' }) return value

        return buildString {
            append('"')
            value.forEach { char ->
                when (char) {
                    '"' -> append("\\\"")
                    else -> append(char)
                }
            }
            append('"')
        }
    }
}
