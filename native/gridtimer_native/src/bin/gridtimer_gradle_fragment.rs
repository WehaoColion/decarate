use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("gradle fragment generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let output_path = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing output path"))?;

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(output_path, FRAGMENT)
}

const FRAGMENT: &str = r#"import groovy.json.JsonSlurper
import org.gradle.api.GradleException
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

repositories { google(); mavenCentral() }

def r = rootProject.file('native/gridtimer_native')
def j = layout.buildDirectory.dir('rustJniLibs')
def g = layout.buildDirectory.dir('generated/source/rustAndroid/main')
def br = file(System.getenv('GRIDTIMER_BUILD_ROOT') ?: 'C:\\gt\\gridtimer-build')
def t = new File(br, 'rustTarget')
def s = new File(br, 'rustSourcegenTarget')
def e = new File(br, 'rustBuildEnvTarget')
def tmp = new File(br, 'rustTmp')
def ch = System.getenv('CARGO_HOME') ?: 'C:\\tools\\cargo'
def rh = System.getenv('RUSTUP_HOME') ?: 'C:\\tools\\rustup'
def c = { a, w, env = [:], o = null ->
    tmp.mkdirs()
    exec {
        workingDir w
        environment [CARGO_HOME: ch, RUSTUP_HOME: rh, TMP: tmp.absolutePath, TEMP: tmp.absolutePath, TMPDIR: tmp.absolutePath] + env
        commandLine a
        if (o != null) standardOutput = o
    }
}
def f = { o, a -> if (o.exists()) { if (o.list()) return; o.deleteDir() }; o.mkdirs(); a(o) }
def b = {
    def o = new ByteArrayOutputStream()
    c(["${ch}\\bin\\cargo.exe", 'run', '-q', '--bin', 'gridtimer_build_env'], r, [CARGO_TARGET_DIR: e.absolutePath], o)
    new JsonSlurper().parseText(o.toString('UTF-8'))
}()

def sdk = b.androidSdk ? file(b.androidSdk) : null
def ndk = b.ndkDir ? file(b.ndkDir) : null
def signing = b.signing
def cargo = b.cargoExe
def linker = b.linker
def lib = b.libDir
def xiaomiAppId = b.xiaomiAppId ?: ''

def verifyRustInputs = { needsNdk = false ->
    if (!r.exists()) throw new GradleException("No crate: ${r.absolutePath}.")
    if (!file(cargo).exists()) throw new GradleException("No Cargo: ${cargo}.")
    if (!lib) throw new GradleException('No xwin.')
    if (needsNdk && !ndk) throw new GradleException("No NDK under ${sdk?.absolutePath ?: 'SDK'}.")
}

android {
    namespace b.namespace
    compileSdk b.compileSdk
    defaultConfig {
        applicationId b.applicationId
        minSdk b.minSdk
        targetSdk b.targetSdk
        versionCode b.versionCode
        versionName b.versionName
        testInstrumentationRunner 'androidx.test.runner.AndroidJUnitRunner'
        manifestPlaceholders.xiaomiAppId = xiaomiAppId
        manifestPlaceholders.xiaomiBuildTypeDebug = true
        vectorDrawables { useSupportLibrary true }
    }
    signingConfigs {
        if (signing) release {
            storeFile rootProject.file(signing.f)
            storePassword signing.p
            keyAlias signing.a
            keyPassword signing.k
            enableV1Signing true
            enableV2Signing true
            enableV3Signing true
            enableV4Signing true
        }
    }
    buildTypes {
        debug {
            applicationIdSuffix '.debug'
            versionNameSuffix '-debug'
            resValue 'string', 'app_name', b.debugAppName
        }
        xiaomiDebug {
            initWith debug
            applicationIdSuffix ''
            versionNameSuffix '-xiaomiDebug'
        }
        release {
            minifyEnabled false
            manifestPlaceholders.xiaomiBuildTypeDebug = false
            if (signing) signingConfig signingConfigs.release
        }
    }
    compileOptions {
        sourceCompatibility JavaVersion.VERSION_17
        targetCompatibility JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = '17' }
    buildFeatures { compose true }
    sourceSets.main {
        def x = g.get().asFile
        manifest.srcFile new File(x, 'AndroidManifest.xml')
        res.srcDir new File(x, 'res')
        java.srcDir x
        jniLibs.srcDir j
    }
    composeOptions { kotlinCompilerExtensionVersion b.composeCompilerExtensionVersion }
    packaging { resources { excludes += '/META-INF/{AL2.0,LGPL2.1}' } }
}

def src = tasks.register('generateAndroidSourcesFromRust') {
    inputs.dir r
    outputs.dir g
    doLast {
        f(g.get().asFile) { o ->
            verifyRustInputs()
            def u = s
            u.mkdirs()
            c([cargo, 'run', '-q', '--bin', 'gridtimer_sourcegen', '--', o.absolutePath], r, [CARGO_TARGET_DIR: u.absolutePath, CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER: linker, LIB: lib])
        }
    }
}
tasks.withType(KotlinCompile).configureEach { dependsOn src }
def nat = tasks.register('buildRustNative') {
    inputs.dir r
    outputs.dir j
    doLast {
        f(j.get().asFile) { o ->
            verifyRustInputs(true)
            def u = t
            u.mkdirs()
            c([cargo, 'ndk', '-o', o.absolutePath, '-t', 'arm64-v8a', '-t', 'armeabi-v7a', '-t', 'x86_64', '-P', '26', 'build', '--release'], r, [CARGO_TARGET_DIR: u.absolutePath, CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER: linker, ANDROID_NDK_HOME: ndk.absolutePath, ANDROID_NDK_ROOT: ndk.absolutePath, ANDROID_NDK: ndk.absolutePath, LIB: lib])
        }
    }
}
tasks.named('preBuild') { dependsOn src, nat }
dependencies {
    def i = { implementation it }; def d = { debugImplementation it }; def a = { androidTestImplementation it }; def p = { platform it }
    i p("androidx.compose:compose-bom:${b.composeBomVersion}")
    a p("androidx.compose:compose-bom:${b.composeBomVersion}")
    b.implementationDeps.each(i)
    b.debugDeps.each(d)
    b.androidTestDeps.each(a)
    b.testDeps.each { testImplementation it }
}
"#;
