# Android verification commands (Kotlin compile + JVM unit tests)
Run from: /home/user/kiwix-tauri/src-tauri/gen/android
Env: ANDROID_HOME=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/sdk JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 GRADLE_USER_HOME=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/gradle-home
Gradle: /tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/gradle-dist/gradle-9.6.1/bin/gradle --no-daemon --offline --max-workers=2 --console=plain

Unit tests (42 pass at baseline):
  cd /home/user/kiwix-tauri/src-tauri/gen/android && ANDROID_HOME=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/sdk JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64 GRADLE_USER_HOME=/tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/gradle-home /tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/gradle-dist/gradle-9.6.1/bin/gradle --no-daemon --offline --max-workers=2 --console=plain :tauri-plugin-kiwix-mobile:testDebugUnitTest --rerun
  results: src-tauri/plugins/tauri-plugin-kiwix-mobile/android/build/test-results/testDebugUnitTest/*.xml

Compile plugin main Kotlin + app module (MainActivity) without cargo:
  (same prefix) ... -I /tmp/claude-0/-home-user-kiwix-tauri/1ea23844-76fe-5181-9a83-b5808a5c6755/scratchpad/android/scripts/app-generated-kotlin.init.gradle :tauri-plugin-kiwix-mobile:compileDebugKotlin :app:compileUniversalDebugKotlin

Notes: --offline works because deps are cached; do not add dependencies. Maven Central rate-limits (429) online runs.
The generated, git-ignored files src-tauri/gen/android/tauri.settings.gradle and app/tauri.build.gradle.kts were hand-written for this container; do not edit or commit them.
