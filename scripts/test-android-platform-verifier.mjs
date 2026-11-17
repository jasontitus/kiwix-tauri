// Guards the Android JVM half of rustls-platform-verifier, which librqbit's
// reqwest 0.13 HTTPS tracker client needs (src-tauri/src/android_tls.rs).
// Upstream requires the Kotlin component to match the Rust crate exactly; a
// Cargo update that moves either crate must fail here until it is re-vendored.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../src-tauri/", import.meta.url);
const vendorName = "rustls-platform-verifier-android-0.2.0";
const vendor = new URL(`vendor/${vendorName}/`, root);
const read = (url) => readFile(url, "utf8");

function lockedVersions(lock, name) {
  return [...lock.matchAll(/\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"/g)]
    .filter((match) => match[1] === name)
    .map((match) => match[2]);
}

test("vendored Kotlin component matches the locked Rust crates", async () => {
  const lock = await read(new URL("Cargo.lock", root));
  const provenance = JSON.parse(await read(new URL("upstream-provenance.json", vendor)));
  assert.deepEqual(lockedVersions(lock, "rustls-platform-verifier-android"), [provenance.version]);
  assert.equal(vendorName, `${provenance.crate}-${provenance.version}`);
  const [crate, version] = provenance.rustCrate.split(" ");
  assert.deepEqual(lockedVersions(lock, crate), [version]);
  // The app initializes the verifier with jni 0.22 types; it must be the
  // verifier's own jni, not a second copy.
  const verifier = lock.split("[[package]]").find((entry) =>
    entry.includes(`name = "${crate}"\nversion = "${version}"`));
  // Cargo writes a bare "jni" once only one version remains in the lock.
  assert.match(verifier, /\n "jni( 0\.22\.4)?",\n/);
  assert.ok(lockedVersions(lock, "jni").includes("0.22.4"));
});

test("vendored upstream files are byte-identical to the recorded commit", async () => {
  const provenance = JSON.parse(await read(new URL("upstream-provenance.json", vendor)));
  assert.match(provenance.commit, /^[0-9a-f]{40}$/);
  // "Unchanged" means equal to the upstream digests recorded at vendoring time,
  // not merely self-consistent: editing a file and its digest together fails.
  const upstreamPath = {
    "src/main/java/org/rustls/platformverifier/CertificateVerifier.kt":
      "src/main/java/org/rustls/platformverifier/CertificateVerifier.kt",
    "upstream/network_security_config.xml": "src/main/res/xml/network_security_config.xml",
  };
  // Pinned here too, so editing every digest in the provenance file still fails.
  const pinned = {
    "src/main/java/org/rustls/platformverifier/CertificateVerifier.kt":
      "251031aec27214398328c4755744cd3274c54e9d51e10952712bcd5ba64bc4ed",
    "upstream/network_security_config.xml":
      "a81434f9139f3780412c358d1cc782fe25339182d3dc141b35413f23d2b29f6a",
  };
  for (const [vendored, upstream] of Object.entries(upstreamPath)) {
    assert.equal(provenance.vendoredUnchanged[vendored], provenance.upstreamModuleSha256[upstream], vendored);
    assert.equal(provenance.vendoredUnchanged[vendored], pinned[vendored], vendored);
  }
  for (const [path, digest] of Object.entries(provenance.vendoredUnchanged)) {
    const actual = createHash("sha256").update(await readFile(new URL(path, vendor))).digest("hex");
    assert.equal(actual, digest, path);
  }
});

// Android's revocation check fails a certificate without an OCSP responder
// (for example current Let's Encrypt leaves) unless its HTTP CRL can be
// fetched. Reproduced on an API 24 emulator: with the app's HTTPS-only policy a
// CRL-only chain verified as Revoked; with the CRL host permitted it passed.
test("the app permits cleartext to exactly upstream's CA revocation hosts", async () => {
  const domains = (xml) => [...xml.matchAll(/<domain includeSubdomains="(true|false)">([^<]+)<\/domain>/g)]
    .map((match) => `${match[2]}|${match[1]}`);
  const upstream = domains(await read(new URL("upstream/network_security_config.xml", vendor)));
  assert.equal(upstream.length, 376);
  const config = await read(new URL("gen/android/app/src/main/res/xml/network_security_config.xml", root));
  assert.match(config, /<base-config cleartextTrafficPermitted="false" \/>/);
  const blocks = [...config.matchAll(/<domain-config cleartextTrafficPermitted="true">([\s\S]*?)<\/domain-config>/g)]
    .map((match) => domains(match[1]));
  assert.deepEqual(blocks, [["localhost|true", "127.0.0.1|false"], upstream]);
  // No trust anchors, pins or debug overrides ride along with the merge.
  assert.doesNotMatch(config, /trust-anchors|pin-set|debug-overrides/);
  const manifest = await read(new URL("gen/android/app/src/main/AndroidManifest.xml", root));
  assert.match(manifest, /android:networkSecurityConfig="@xml\/network_security_config"/);
});

