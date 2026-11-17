# Rust loads these classes and reads VerificationResult's `code`/`message`
# fields through JNI only; R8 cannot see those uses. This is the rule upstream's
# README prescribes for consuming applications. Upstream's own
# `-dontobfuscate` is deliberately not exported: it would apply app-wide.
-keep, includedescriptorclasses class org.rustls.platformverifier.** { *; }
