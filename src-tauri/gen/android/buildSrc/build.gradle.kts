plugins {
    `kotlin-dsl`
}

gradlePlugin {
    plugins {
        create("pluginsForCoolKids") {
            id = "rust"
            implementationClass = "RustPlugin"
        }
    }
}

repositories {
    google()
    mavenCentral()
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    compileOnly(gradleApi())
    implementation("com.android.tools.build:gradle:9.3.1")
}

