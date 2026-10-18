import java.io.File
import java.util.zip.ZipFile
import java.util.zip.ZipInputStream
import javax.xml.XMLConstants
import javax.xml.parsers.DocumentBuilderFactory
import org.gradle.api.artifacts.component.ModuleComponentIdentifier
import org.gradle.api.artifacts.result.ComponentArtifactsResult
import org.gradle.api.artifacts.result.ResolvedArtifactResult
import org.gradle.api.artifacts.result.UnresolvedArtifactResult
import org.gradle.api.artifacts.result.UnresolvedComponentResult
import org.gradle.maven.MavenModule
import org.gradle.maven.MavenPomArtifact
import org.gradle.process.CommandLineArgumentProvider
import org.w3c.dom.Element

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

// Shown on the licences screen and written into the licence list.
val sourceCodeUrl = "https://github.com/perritodeck/perritodeck"

android {
    namespace = "dev.perritodeck.client"
    compileSdk = 36
    // AGP 8.11's default, written out so CI, F-Droid and local builds use the same one.
    // Raise it when AGP is upgraded.
    buildToolsVersion = "35.0.0"

    defaultConfig {
        // Never change after the first public release.
        applicationId = "dev.perritodeck.client"
        minSdk = 22
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        buildConfigField("String", "SOURCE_URL", "\"$sourceCodeUrl\"")
    }

    // Release signing comes from the environment (CI secrets or a local keystore). Without
    // PERRITO_KEYSTORE (unset or blank), release builds are unsigned, which is what F-Droid
    // expects, since it signs builds itself. With it, the other three must be set as well: a
    // half-set group stops the build here instead of producing a wrongly signed APK.
    fun env(name: String): String? = System.getenv(name)?.takeIf { it.isNotBlank() }
    val keystorePath = env("PERRITO_KEYSTORE")
    if (keystorePath != null) {
        val missing = listOf("PERRITO_KEYSTORE_PASSWORD", "PERRITO_KEY_ALIAS", "PERRITO_KEY_PASSWORD")
            .filter { env(it) == null }
        if (missing.isNotEmpty()) {
            val names = if (missing.size == 1) missing[0] else missing.dropLast(1).joinToString() + " and " + missing.last()
            throw GradleException(
                "PERRITO_KEYSTORE is set, but $names ${if (missing.size == 1) "is" else "are"} not. " +
                    "Set all four to sign a release, or clear PERRITO_KEYSTORE to build it unsigned.",
            )
        }
        val keystore = file(keystorePath)
        if (!keystore.isFile) {
            throw GradleException("PERRITO_KEYSTORE points at $keystore, which doesn't exist.")
        }
        signingConfigs {
            create("release") {
                storeFile = keystore
                storePassword = env("PERRITO_KEYSTORE_PASSWORD")
                keyAlias = env("PERRITO_KEY_ALIAS")
                keyPassword = env("PERRITO_KEY_PASSWORD")
                // v1 (JAR signing) is what Android 5.1 and 6 check, v2 is what 7 and newer check.
                // v3 (key rotation) stays off (D7 in docs/plan/phase-8.md); see docs/releasing.md.
                enableV1Signing = true
                enableV2Signing = true
                enableV3Signing = false
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // AGP would write the git commit into release APKs
            // (META-INF/version-control-info.textproto), so a build from a source tarball, or
            // one without .git, wouldn't match the published APK byte for byte.
            vcsInfo { include = false }
        }
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    // F-Droid flags the encrypted dependency-info block Google adds to APKs.
    dependenciesInfo {
        includeInApk = false
        includeInBundle = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    val fixturesDir = rootProject.file("../protocol/fixtures").absolutePath
    // LauncherIconTest compares the icon with its sources in docs/assets.
    val repoDir = rootProject.file("..").absolutePath
    testOptions {
        unitTests.all {
            it.systemProperty("perritodeck.fixtures", fixturesDir)
            it.systemProperty("perritodeck.repo", repoDir)
            // The tests read these files themselves, so they're inputs of the test task as well.
            // Otherwise a change to one leaves the task up to date, or takes its result from the
            // build cache, and the tests don't run.
            it.inputs.dir(fixturesDir).withPropertyName("protocolFixtures").withPathSensitivity(PathSensitivity.RELATIVE)
            it.inputs.files(fileTree("$repoDir/docs/assets") { include("*.svg", "android/**") })
                .withPropertyName("iconSources").withPathSensitivity(PathSensitivity.RELATIVE)
            it.inputs.dir("src/main/res").withPropertyName("appResources").withPathSensitivity(PathSensitivity.RELATIVE)
        }
    }

    lint {
        abortOnError = true
        warningsAsErrors = false
    }
}

// Release APKs carry the prebuilt AndroidX native libraries (graphics-path and CameraX) exactly
// as they come. AGP strips them only when an NDK is installed: GitHub's runners have one and
// F-Droid's build server doesn't, so stripping would make the two builds differ. Debug builds
// are left as they were.
androidComponents {
    onVariants(selector().withBuildType("release")) { variant ->
        variant.packaging.jniLibs.keepDebugSymbols.add("**/*.so")
    }

    // Every variant's APK carries assets/third-party-licenses.txt, listing the libraries
    // packaged into that variant (see ThirdPartyLicensesTask below). The unit tests read it
    // through the perritodeck.licenses system property.
    onVariants { variant ->
        val suffix = variant.name.replaceFirstChar { it.uppercase() }
        val artifacts = variant.runtimeConfiguration.incoming.artifacts.resolvedArtifacts
        val generate = tasks.register<ThirdPartyLicensesTask>("generate${suffix}ThirdPartyLicenses") {
            artifactFiles.set(
                artifacts.map { results ->
                    results.map { r ->
                        val id = r.id.componentIdentifier
                        val coordinates = if (id is ModuleComponentIdentifier) "${id.group}:${id.module}:${id.version}" else "?"
                        "$coordinates\t${r.file.absolutePath}"
                    }
                },
            )
            libraries.set(artifactFiles.map { lines -> lines.map { it.substringBefore('\t') }.distinct().sorted() })
            pomPaths.set(providers.provider { resolvePoms(libraries.get()) })
            apacheLicense.set(rootProject.file("../LICENSE"))
            otherLicenses.set(layout.projectDirectory.dir("licenses"))
            sourceUrl.set(sourceCodeUrl)
        }
        variant.sources.assets?.addGeneratedSourceDirectory(generate, ThirdPartyLicensesTask::outputDir)

        val asset = objects.newInstance<LicensesAssetArgument>()
        asset.dir.set(generate.flatMap { it.outputDir })
        tasks.withType<Test>().configureEach {
            if (name == "test${suffix}UnitTest") jvmArgumentProviders.add(asset)
        }
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.compose.ui.tooling.preview)
    debugImplementation(libs.compose.ui.tooling)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.okhttp)
    // QR scanning: CameraX + ZXing (both Apache-2.0, no Google Play Services).
    implementation(libs.camerax.camera2)
    implementation(libs.camerax.lifecycle)
    implementation(libs.camerax.view)
    implementation(libs.zxing.core)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.okhttp.mockwebserver)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    debugImplementation(libs.compose.ui.test.manifest)
}

/**
 * "group:name:version<tab>path" for the POM of each of [libraries] and of every parent POM above
 * them, sorted. Gradle resolves them like any other dependency, so it downloads a POM again if
 * its cache cleanup has deleted it. That happens: the cleanup deletes files Gradle hasn't used
 * for 30 days, and once Gradle has a module's metadata it builds without reading the POM.
 *
 * It runs when the configuration cache stores the task, or when the task runs if there's no
 * cache. Resolving earlier, while the task graph is built, makes AGP warn.
 */
fun resolvePoms(libraries: List<String>): List<String> {
    val found = sortedMapOf<String, File>()
    var wanted = libraries.filter { it != "?" }.toSet()
    while (wanted.isNotEmpty()) {
        val query = dependencies.createArtifactResolutionQuery()
        for (coordinates in wanted) {
            val (group, module, version) = coordinates.split(':')
            query.forModule(group, module, version)
        }
        val result = query.withArtifacts(MavenModule::class.java, MavenPomArtifact::class.java).execute()
        val added = result.components.map { component ->
            val id = component.id
            val coordinates = if (id is ModuleComponentIdentifier) "${id.group}:${id.module}:${id.version}" else id.displayName
            val pom = when (component) {
                is UnresolvedComponentResult -> throw GradleException("Couldn't resolve $coordinates to read its POM.", component.failure)
                is ComponentArtifactsResult -> component.getArtifacts(MavenPomArtifact::class.java).singleOrNull()
                else -> null
            }
            when (pom) {
                is ResolvedArtifactResult -> coordinates to pom.file
                is UnresolvedArtifactResult -> throw GradleException("Couldn't download the POM of $coordinates.", pom.failure)
                else -> throw GradleException("Gradle found no POM for $coordinates.")
            }
        }
        found += added
        wanted = added.mapNotNull { (_, pom) -> ThirdPartyLicensesTask.parentOf(pom) }.toSet() - found.keys
    }
    return found.map { (coordinates, pom) -> "$coordinates\t${pom.absolutePath}" }
}

/**
 * Writes third-party-licenses.txt for one variant's assets: every library packaged into that
 * APK (its resolved runtime classpath), the licences its POM declares (inherited from a parent
 * POM as Maven does), any NOTICE file it carries, and then the licence texts. The licences
 * screen shows it (ui/LicensesScreen.kt, parsed by data/ThirdPartyLicenses.kt).
 *
 * The POMs come from Gradle (resolvePoms above), which downloads them like any dependency. A
 * library under a licence with no text here stops the build, so a new dependency can't ship
 * without its licence. The output depends only on the dependency versions and the texts, never
 * on the machine or the time, so the APK stays reproducible.
 */
abstract class ThirdPartyLicensesTask : DefaultTask() {
    /** group:name:version of every packaged library, sorted. */
    @get:Input
    abstract val libraries: ListProperty<String>

    /**
     * "group:name:version<tab>path" for each library file, to find its NOTICE files.
     * The paths differ between machines but a released version's files don't, so only
     * [libraries] decides whether the task is up to date.
     */
    @get:Internal
    abstract val artifactFiles: ListProperty<String>

    /**
     * "group:name:version<tab>path" for the POM of each library and of every parent POM above
     * it, which the licences come from. Like [artifactFiles], they don't decide whether the task
     * is up to date. (As input files, Gradle would resolve them before the task graph is ready,
     * which AGP warns about.)
     */
    @get:Internal
    abstract val pomPaths: ListProperty<String>

    /** The Apache License 2.0 text: the repository's LICENSE, which is Perrito's own licence. */
    @get:InputFile
    @get:PathSensitive(PathSensitivity.NONE)
    abstract val apacheLicense: RegularFileProperty

    /** The texts of the other licences, one file for each entry in [OTHER_LICENSES] and [UNDECLARED_LICENSES]. */
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val otherLicenses: DirectoryProperty

    @get:Input
    abstract val sourceUrl: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    private class Library(
        val group: String,
        val module: String,
        val version: String,
        val name: String?,
        val url: String?,
        val licenses: List<String>,
        /** Path inside the library to the text of each NOTICE file. */
        val notices: List<Pair<String, String>>,
    ) {
        val title get() = "$group:$module $version"
    }

    @TaskAction
    fun generate() {
        val files = artifactFiles.get().groupBy({ it.substringBefore('\t') }, { File(it.substringAfter('\t')) })
        files["?"]?.let { unknown ->
            throw GradleException(
                "These files in the app don't come from a Maven repository, so their licence can't be " +
                    "read: ${unknown.joinToString { it.name }}",
            )
        }
        val poms = pomPaths.get().associate { it.substringBefore('\t') to File(it.substringAfter('\t')) }
        val libs = libraries.get().map { describe(it, files.getValue(it), poms) }
            .sortedWith(compareBy({ it.group }, { it.module }, { it.version }))

        val out = StringBuilder()
        fun line(s: String = "") = out.append(s).append('\n')
        fun section(title: String) {
            line()
            line(RULE)
            line(title)
            line(DASHES)
        }
        fun body(text: String) {
            val cleaned = clean(text)
            if (cleaned.lines().any { it == RULE }) throw GradleException("A licence text contains a line of 78 '=', which the app reads as a new section.")
            line(cleaned)
        }

        line("Open-source software in Perrito")
        line()
        line("Perrito is free software under the Apache License 2.0. Its source code is at")
        line("${sourceUrl.get()}.")
        line()
        line("Perrito includes the libraries below, each under the licence named in the list.")
        line("The licence texts and the libraries' notices follow the list.")

        section(LIBRARIES)
        libs.forEachIndexed { i, lib ->
            if (i > 0) line()
            line(lib.title)
            lib.name?.let { line(it) }
            line("Licence: ${lib.licenses.joinToString(", ")}")
            lib.url?.let { line(it) }
        }
        for (lib in libs) {
            for ((path, text) in lib.notices) {
                section("Notice in ${lib.title} ($path)")
                body(text)
            }
        }
        val otherLicenseTexts = OTHER_LICENSES.values + UNDECLARED_LICENSES.values.flatten()
        for ((label, fileName) in otherLicenseTexts.distinct().sortedBy { it.first }) {
            val users = libs.filter { label in it.licenses }
            if (users.isEmpty()) continue
            val text = otherLicenses.get().file(fileName).asFile
            if (!text.isFile) throw GradleException("The text of $label should be in $text, which doesn't exist.")
            section("$label, used by ${users.joinToString { it.title }}")
            body(text.readText(Charsets.UTF_8))
        }
        section("$APACHE_TITLE, used by Perrito and the libraries marked $APACHE")
        body(apacheLicense.get().asFile.readText(Charsets.UTF_8))

        val dir = outputDir.get().asFile
        dir.deleteRecursively()
        dir.mkdirs()
        // "\n" on every OS and no date, so every build of the same versions writes the same bytes.
        File(dir, FILE_NAME).writeText(out.toString(), Charsets.UTF_8)
    }

    private fun describe(coordinates: String, files: List<File>, poms: Map<String, File>): Library {
        val (group, module, version) = coordinates.split(':')
        val project = parseXml(pom(coordinates, poms))
        val declared = declaredLicenses(project, coordinates, poms, 0)
        if (declared.isEmpty()) throw GradleException("$coordinates names no licence in its POM or a parent POM.")
        val labels = declared.map { (name, url) ->
            licenseLabel(name, url) ?: throw GradleException(
                "$coordinates is under \"$name\" ($url), and the app has no text for that licence. Put the " +
                    "text in android/app/licenses/ and add it to OTHER_LICENSES in android/app/build.gradle.kts.",
            )
        }
        val undeclared = UNDECLARED_LICENSES["$group:$module"].orEmpty().map { it.first }
        val notices = files.flatMap(::notices).distinctBy { it.first }.sortedBy { it.first }
        return Library(group, module, version, text(project, "name"), text(project, "url"), (labels + undeclared).distinct(), notices)
    }

    /** The licences a POM declares or, when it declares none, the ones its parent declares. */
    private fun declaredLicenses(project: Element, of: String, poms: Map<String, File>, depth: Int): List<Pair<String, String>> {
        val declared = child(project, "licenses")?.let { children(it, "license") }.orEmpty()
            .map { (text(it, "name") ?: "") to (text(it, "url") ?: "") }
        if (declared.isNotEmpty()) return declared
        val parent = parentOf(project) ?: return emptyList()
        if (depth > 10) return emptyList()
        return declaredLicenses(parseXml(pom(parent, poms, of)), of, poms, depth + 1)
    }

    /** The POM Gradle resolved for [coordinates] (the library [of] names it as a parent). */
    private fun pom(coordinates: String, poms: Map<String, File>, of: String? = null): File {
        val what = if (of == null) coordinates else "$coordinates, the parent POM of $of,"
        val file = poms[coordinates] ?: throw GradleException("Gradle resolved no POM for $what so its licence can't be read.")
        if (!file.isFile) throw GradleException("The POM of $what is no longer at $file. $GONE_FROM_CACHE")
        return file
    }

    private fun licenseLabel(name: String, url: String): String? {
        val apacheUrl = Regex("""apache\.org/licenses/LICENSE-2\.0""").containsMatchIn(url)
        val apacheName = Regex("""(the )?apache( software)?( license)?,?( version)?[ -]?2\.0""", RegexOption.IGNORE_CASE).matches(name)
        if (apacheUrl || apacheName) return APACHE
        return OTHER_LICENSES[url]?.first
    }

    /** NOTICE files in a jar or an aar (and the jars inside it), which Apache-2.0 asks to pass on. */
    private fun notices(file: File): List<Pair<String, String>> {
        if (file.extension !in setOf("jar", "aar")) return emptyList()
        // Without the file its notices would be left out of the list without a word.
        if (!file.isFile) throw GradleException("$file, one of the app's libraries, is missing. $GONE_FROM_CACHE")
        val found = mutableListOf<Pair<String, String>>()
        ZipFile(file).use { zip ->
            for (entry in zip.entries().asSequence().filter { !it.isDirectory }.sortedBy { it.name }) {
                if (isNotice(entry.name)) {
                    found += entry.name to zip.getInputStream(entry).use { it.readBytes() }.toString(Charsets.UTF_8)
                } else if (entry.name.endsWith(".jar")) {
                    ZipInputStream(zip.getInputStream(entry)).use { jar ->
                        while (true) {
                            val inner = jar.nextEntry ?: break
                            if (!inner.isDirectory && isNotice(inner.name)) found += inner.name to jar.readBytes().toString(Charsets.UTF_8)
                        }
                    }
                }
            }
        }
        return found
    }

    private fun isNotice(path: String) = path.substringAfterLast('/').uppercase() in setOf("NOTICE", "NOTICE.TXT", "NOTICE.MD")

    /** Unix line ends, no trailing spaces, no blank lines at either end; indentation kept. */
    private fun clean(text: String): String =
        text.replace("\r\n", "\n").replace('\r', '\n').lines().map { it.trimEnd() }
            .dropWhile { it.isEmpty() }.dropLastWhile { it.isEmpty() }.joinToString("\n")

    companion object {
        const val FILE_NAME = "third-party-licenses.txt"
        const val LIBRARIES = "Libraries"
        const val APACHE = "Apache-2.0"
        const val APACHE_TITLE = "Apache License 2.0"
        val RULE = "=".repeat(78)
        val DASHES = "-".repeat(78)

        // A reused configuration cache entry keeps the paths Gradle resolved when it was stored,
        // even after Gradle's cache cleanup deletes the files (after 30 days without a
        // resolution). Resolving again downloads them to the same place.
        const val GONE_FROM_CACHE = "Gradle's cache cleanup has probably deleted it since the " +
            "configuration cache was stored. Run the build once with --no-configuration-cache, " +
            "which downloads it again."

        /**
         * Licences other than Apache-2.0, by the URL a POM gives for them: the label shown in the
         * list, and the file in app/licenses/ that holds the text.
         */
        val OTHER_LICENSES = mapOf(
            // camera-core's native image code (libimage_processing_util_jni.so) includes libyuv.
            // The text is libyuv's LICENSE, as AOSP ships it in external/libyuv.
            "https://chromium.googlesource.com/libyuv/libyuv/+/refs/heads/main/README.chromium" to
                ("BSD-3-Clause (libyuv)" to "libyuv.txt"),
        )

        /**
         * Licences for part of a library that its POM doesn't name, by group:name, written like
         * [OTHER_LICENSES]' values.
         */
        val UNDECLARED_LICENSES = mapOf(
            // OkHttp carries the Public Suffix List (okhttp3/internal/publicsuffix), which is
            // MPL-2.0. The text is the MPL's, from mozilla.org/MPL/2.0/.
            "com.squareup.okhttp3:okhttp" to listOf("MPL-2.0 (Public Suffix List data)" to "mpl-2.0.txt"),
        )

        /** group:name:version of the parent the POM in [pom] names, if it names one. */
        fun parentOf(pom: File): String? = parentOf(parseXml(pom))

        private fun parentOf(project: Element): String? {
            val parent = child(project, "parent") ?: return null
            val g = text(parent, "groupId") ?: return null
            val a = text(parent, "artifactId") ?: return null
            val v = text(parent, "version") ?: return null
            return "$g:$a:$v"
        }

        private fun parseXml(file: File): Element {
            val factory = DocumentBuilderFactory.newInstance()
            factory.setFeature(XMLConstants.FEATURE_SECURE_PROCESSING, true)
            factory.setFeature("http://apache.org/xml/features/nonvalidating/load-external-dtd", false)
            factory.isExpandEntityReferences = false
            return factory.newDocumentBuilder().parse(file).documentElement
        }

        private fun children(parent: Element, tag: String): List<Element> {
            val nodes = parent.childNodes
            return (0 until nodes.length).map { nodes.item(it) }.filterIsInstance<Element>().filter { it.tagName == tag }
        }

        private fun child(parent: Element, tag: String): Element? = children(parent, tag).firstOrNull()

        /** A POM element's text with its whitespace collapsed, or null when it's missing or a ${placeholder}. */
        private fun text(parent: Element, tag: String): String? =
            child(parent, tag)?.textContent?.trim()?.replace(Regex("""\s+"""), " ")?.takeIf { it.isNotEmpty() && "\${" !in it }
    }
}

/** Hands a variant's unit tests the path of the licence list it generated. */
abstract class LicensesAssetArgument : CommandLineArgumentProvider {
    @get:InputDirectory
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val dir: DirectoryProperty

    override fun asArguments(): Iterable<String> =
        listOf("-Dperritodeck.licenses=" + dir.get().file(ThirdPartyLicensesTask.FILE_NAME).asFile.absolutePath)
}
