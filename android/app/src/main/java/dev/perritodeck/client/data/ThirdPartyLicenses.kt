package dev.perritodeck.client.data

/**
 * Reads the licence list the build puts in the app's assets (ThirdPartyLicensesTask in
 * app/build.gradle.kts): a preamble, then sections that each start with a line of 78 "=", a
 * title and a line of 78 "-". The first section lists the libraries, one paragraph each; the
 * others hold notices and licence texts.
 */
object ThirdPartyLicenses {
    const val ASSET = "third-party-licenses.txt"
    const val LIBRARIES = "Libraries"
    private val RULE = "=".repeat(78)
    private val DASHES = "-".repeat(78)

    /** [paragraphs] are blocks of text between blank lines, each as its lines. */
    data class Section(val title: String, val paragraphs: List<List<String>>)

    /** A library from the list: "group:name version", then its name, licence and web page. */
    data class Library(val title: String, val details: List<String>)

    data class Document(val preamble: List<List<String>>, val sections: List<Section>) {
        val libraries: List<Library>
            get() = sections.firstOrNull { it.title == LIBRARIES }?.paragraphs.orEmpty()
                .map { Library(it.first(), it.drop(1)) }

        /** Notices and licence texts: everything after the library list. */
        val texts: List<Section> get() = sections.filter { it.title != LIBRARIES }
    }

    /** How a paragraph of licence text is shown. */
    data class Paragraph(val text: String, val indent: Int, val centered: Boolean, val heading: Boolean = false)

    fun parse(text: String): Document {
        val lines = text.replace("\r\n", "\n").split('\n')
        var i = 0
        val preamble = mutableListOf<String>()
        while (i < lines.size && lines[i] != RULE) preamble += lines[i++]
        val sections = mutableListOf<Section>()
        while (i < lines.size) {
            val title = lines.getOrElse(i + 1) { "" }.trim()
            var j = i + 2
            if (lines.getOrNull(j) == DASHES) j++
            val body = mutableListOf<String>()
            while (j < lines.size && lines[j] != RULE) body += lines[j++]
            sections += Section(title, paragraphs(body))
            i = j
        }
        return Document(paragraphs(preamble), sections)
    }

    /** Blocks of text between blank lines. A box drawn with "*" (the MPL's disclaimers) is left out, keeping what's inside. */
    fun paragraphs(lines: List<String>): List<List<String>> {
        val out = mutableListOf<List<String>>()
        var current = mutableListOf<String>()
        for (line in lines.map(::unboxed)) {
            if (line.isBlank()) {
                if (current.isNotEmpty()) out += current
                current = mutableListOf()
            } else {
                current += line.trimEnd()
            }
        }
        if (current.isNotEmpty()) out += current
        return out
    }

    private fun unboxed(line: String): String {
        val trimmed = line.trimEnd()
        return when {
            trimmed.length < 2 || !trimmed.startsWith('*') || !trimmed.endsWith('*') -> line
            trimmed.all { it == '*' } -> ""
            else -> trimmed.substring(1, trimmed.length - 1)
        }
    }

    /**
     * Licence texts are wrapped at about 70 characters for a terminal. On a screen the lines of
     * a paragraph are joined so they wrap to its width, keeping the first line's indentation.
     * A paragraph whose every line is indented ten spaces or more is a centred heading (the top
     * of the Apache License), and keeps its lines. A line underlined with "=" or "-" is a
     * heading (the MPL's sections), shown without the underline.
     */
    fun reflow(lines: List<String>): Paragraph {
        val underline = Regex("""\s*(=+|-+)\s*""")
        if (lines.size == 2 && underline.matches(lines[1])) {
            return Paragraph(lines[0].trim(), indent = 0, centered = false, heading = true)
        }
        val indents = lines.map { it.length - it.trimStart().length }
        return if (indents.all { it >= 10 }) {
            Paragraph(lines.joinToString("\n") { it.trim() }, indent = 0, centered = true)
        } else {
            Paragraph(lines.joinToString(" ") { it.trim() }, indent = indents.firstOrNull() ?: 0, centered = false)
        }
    }
}
