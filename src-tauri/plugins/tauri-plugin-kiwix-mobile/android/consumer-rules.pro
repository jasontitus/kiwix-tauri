-keep class org.kiwix.mobile.KiwixMobilePlugin { *; }
-keep class org.kiwix.mobile.KiwixTransferService { *; }
-keep @app.tauri.annotation.InvokeArg class org.kiwix.mobile.** { *; }
-keepclassmembers class org.kiwix.mobile.** {
    @app.tauri.annotation.Command <methods>;
    @app.tauri.annotation.ActivityCallback <methods>;
    @app.tauri.annotation.PermissionCallback <methods>;
}
