package com.konvoy.ide.run

import junit.framework.TestCase
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

class KonvoyPluginXmlTest : TestCase() {

    fun testRunLineMarkerContributorsUseImplementationClass() {
        val descriptor = pluginDescriptor()
        val contributors = descriptor.getElementsByTagName("runLineMarkerContributor")

        assertTrue("expected runLineMarkerContributor entries", contributors.length > 0)
        for (index in 0 until contributors.length) {
            val contributor = contributors.item(index)
            val attributes = contributor.attributes
            assertNotNull(
                "runLineMarkerContributor must use implementationClass",
                attributes.getNamedItem("implementationClass"),
            )
            assertNull(
                "runLineMarkerContributor must not use implementation",
                attributes.getNamedItem("implementation"),
            )
        }
    }

    private fun pluginDescriptor() =
        DocumentBuilderFactory.newInstance()
            .newDocumentBuilder()
            .parse(File("src/main/resources/META-INF/plugin.xml"))
}
