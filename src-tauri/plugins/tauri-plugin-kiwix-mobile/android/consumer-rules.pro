-keep class org.kiwix.mobile.KiwixMobilePlugin { *; }
-keep class org.kiwix.mobile.KiwixTransferService { *; }
-keep @app.tauri.annotation.InvokeArg class org.kiwix.mobile.** { *; }
-keepclassmembers class org.kiwix.mobile.** {
    @app.tauri.annotation.Command <methods>;
    @app.tauri.annotation.ActivityCallback <methods>;
    @app.tauri.annotation.PermissionCallback <methods>;
}
# Rust resolves these by JNI symbol name (Java_org_kiwix_mobile_<Class>_<method>):
# NativeArticlePdf.authorityCurrent/commitHandoff and
# KiwixMobilePlugin.installServiceEndpoint. Renaming the class or the method
# leaves the symbol unresolved at first use. The Android Gradle plugin's default
# rules keep native names too, but this library must not depend on a consumer
# keeping those defaults enabled.
-keepclasseswithmembernames,includedescriptorclasses class org.kiwix.mobile.** {
    native <methods>;
}
