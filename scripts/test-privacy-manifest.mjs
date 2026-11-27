import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import test from "node:test";

// App Store Connect rejects an upload whose code calls a "required reason" API
// that the bundled privacy manifest does not declare (ITMS-91053). Nothing in
// the build checks that, so this compares the manifest with the first-party
// native sources. Rust and third-party frameworks are not scanned: the Rust
// side only ever adds categories, and each SDK ships its own manifest.

const root = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, root), "utf8");
const list = (directory, extension) => readdirSync(new URL(directory, root))
  .filter((name) => name.endsWith(extension)).sort().map((name) => directory + name);

const manifestPath = "src-tauri/PrivacyInfo.xcprivacy";
const projectPath = "src-tauri/gen/apple/project.yml";
const swiftSources = "src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Sources/";
const objcSources = "src-tauri/native/";
const extensionSources = "src-tauri/gen/apple/DownloadActivity/";
// Compiled into the widget extension as well as the application (project.yml).
const sharedWithExtension = [
  swiftSources + "DownloadActivityAttributes.swift",
  swiftSources + "ReadAloudActivityAttributes.swift",
];

// Apple's required-reason API families, the symbols that put code in each one,
// and the reason codes an application may declare for it. Apple's two codes
// that only a third-party SDK may declare (0A2A.1, C56D.1) are left out.
const families = {
  NSPrivacyAccessedAPICategoryFileTimestamp: {
    reasons: ["DDA9.1", "C617.1", "3B52.1"],
    patterns: [
      /\b(?:l|f)?stat(?:at)?\s*\(/,
      /\bf?getattrlist(?:bulk|at)?\b/,
      /\bst_(?:mtimespec|ctimespec|atimespec|birthtimespec|mtime|ctime|birthtime)\b/,
      /\b(?:NSURL)?(?:creationDate|CreationDate|contentModificationDate|ContentModificationDate|attributeModificationDate|AttributeModificationDate)(?:Key)?\b/,
      /\b(?:NSFile)?(?:modificationDate|ModificationDate|fileModificationDate|fileCreationDate)\b/,
    ],
  },
  NSPrivacyAccessedAPICategorySystemBootTime: {
    reasons: ["35F9.1", "8FFB.1", "3D61.1"],
    patterns: [/\bsystemUptime\b/, /\bmach_absolute_time\b/],
  },
  NSPrivacyAccessedAPICategoryDiskSpace: {
    reasons: ["85F4.1", "E174.1", "7D9E.1", "B728.1"],
    patterns: [
      /\b(?:NSURL)?[vV]olumeAvailableCapacity(?:ForImportantUsage|ForOpportunisticUsage)?(?:Key)?\b/,
      /\b(?:NSURL)?[vV]olumeTotalCapacity(?:Key)?\b/,
      /\b(?:NSFileSystemFreeSize|NSFileSystemSize|systemFreeSize|systemSize)\b/,
      /\bf?statv?fs\s*\(/,
    ],
  },
  NSPrivacyAccessedAPICategoryActiveKeyboards: {
    reasons: ["3EC4.1", "54BD.1"],
    patterns: [/\bactiveInputModes\b/],
  },
  NSPrivacyAccessedAPICategoryUserDefaults: {
    reasons: ["CA92.1", "1C8F.1", "AC6B.1"],
    patterns: [/\b(?:NS)?UserDefaults\b/],
  },
};

// Comments describe APIs without calling them. "//" inside a URL literal is
// kept, so code after it on the same line is still scanned.
function code(source) {
  return source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, " "))
    .replace(/(^|[^:"])\/\/.*$/gm, "$1");
}

/** First use of each family: `{ category: "path:line" }`. */
function used(paths, load = read) {
  const found = {};
  for (const path of paths) {
    code(load(path)).split("\n").forEach((line, index) => {
      for (const [category, family] of Object.entries(families)) {
        if (!found[category] && family.patterns.some((pattern) => pattern.test(line))) {
          found[category] = `${path}:${index + 1}`;
        }
      }
    });
  }
  return found;
}

/** The subset of XML property lists a privacy manifest uses: dictionaries,
 * arrays, strings and booleans. Key order and whitespace do not matter. */
function plist(xml) {
  const tokens = [...xml.replace(/<\?xml[\s\S]*?\?>|<!DOCTYPE[\s\S]*?>|<!--[\s\S]*?-->/g, "")
    .matchAll(/<(key|string)>([^<]*)<\/\1>|<(\/?)(plist|dict|array)(?:\s[^>]*)?(\/?)>|<(true|false)\/>|([^<\s][^<]*)/g)];
  let at = 0;
  const value = () => {
    const [text, tag, content, closing, container, empty, flag] = tokens[at++] ?? [];
    assert.ok(text !== undefined, "The privacy manifest ends inside a value");
    if (tag === "string") return content;
    if (flag) return flag === "true";
    assert.ok(container && !closing && tag !== "key", `Unexpected ${text} in the privacy manifest`);
    if (container === "array") {
      const items = [];
      if (empty) return items;
      while (tokens[at]?.[0] !== "</array>") items.push(value());
      at += 1;
      return items;
    }
    const entries = {};
    if (empty) return entries;
    while (tokens[at]?.[0] !== "</dict>") {
      const [key, kind, name] = tokens[at++] ?? [];
      assert.equal(kind, "key", `Expected a key, found ${key} in the privacy manifest`);
      assert.ok(!(name in entries), `${name} appears twice in one dictionary`);
      entries[name] = value();
    }
    at += 1;
    return entries;
  };
  assert.match(tokens[at++]?.[0] ?? "", /^<plist[\s>]/, "The privacy manifest is not a property list");
  const document = value();
  assert.equal(tokens[at++]?.[0], "</plist>", "The privacy manifest has content after its root value");
  assert.equal(at, tokens.length, "The privacy manifest has content after </plist>");
  return document;
}

/** `{ category: [reason, ...] }` from the manifest's accessed-API array. */
function declared(xml) {
  const types = plist(xml).NSPrivacyAccessedAPITypes;
  assert.ok(Array.isArray(types), "The privacy manifest has no NSPrivacyAccessedAPITypes array");
  const entries = {};
  for (const entry of types) {
    const { NSPrivacyAccessedAPIType: type, NSPrivacyAccessedAPITypeReasons: reasons } = entry;
    assert.ok(typeof type === "string" && Array.isArray(reasons),
      `Malformed accessed-API entry: ${JSON.stringify(entry)}`);
    assert.equal(entries[type], undefined, `${type} is declared twice`);
    entries[type] = reasons;
  }
  return entries;
}

test("the detector recognizes each required-reason family and ignores comments", () => {
  const samples = {
    NSPrivacyAccessedAPICategoryFileTimestamp: [
      "guard fstat(fd, &info) == 0 else { return }",
      "let modified = Int64(info.st_mtimespec.tv_sec)",
      "if (lstat(path, &info) != 0) return;",
      "let date = try url.resourceValues(forKeys: [.contentModificationDateKey])",
    ],
    NSPrivacyAccessedAPICategorySystemBootTime: [
      "let now = ProcessInfo.processInfo.systemUptime",
      "uint64_t start = mach_absolute_time();",
    ],
    NSPrivacyAccessedAPICategoryDiskSpace: [
      "let free = try url.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])",
      "if (statfs(path, &buffer) != 0) return;",
    ],
    NSPrivacyAccessedAPICategoryActiveKeyboards: ["let modes = UITextInputMode.activeInputModes"],
    NSPrivacyAccessedAPICategoryUserDefaults: [
      "init(defaults: UserDefaults = .standard) {}",
      "[[NSUserDefaults standardUserDefaults] setObject:value forKey:key];",
    ],
  };
  for (const [category, lines] of Object.entries(samples)) {
    for (const line of lines) {
      assert.deepEqual(Object.keys(used(["sample"], () => line)), [category], line);
    }
  }
  for (const line of [
    "// UserDefaults is not used here; fstat(fd) neither",
    "/* systemUptime and\n   statfs( are only mentioned */",
    "let status = mkstat(value); let restated = 1",
    "let link = \"https://example.org\"",
  ]) {
    assert.deepEqual(used(["sample"], () => line), {}, line);
  }
  // A URL literal must not hide a call that follows it on the same line.
  assert.deepEqual(Object.keys(used(["sample"], () => "open(\"kiwix://x\"); fstat(fd, &info)")),
    ["NSPrivacyAccessedAPICategoryFileTimestamp"]);
});

test("the application privacy manifest declares every required-reason API its native code uses", () => {
  const sources = [...list(swiftSources, ".swift"), ...list(objcSources, ".mm")];
  assert.ok(sources.some((path) => path.endsWith("DocumentStorage.swift")),
    "The iOS plugin sources moved; update this test's paths");
  assert.ok(sources.some((path) => path.endsWith(".mm")), "The native sources moved; update this test's paths");
  const manifest = declared(read(manifestPath));
  const found = used(sources);
  const missing = Object.entries(found).filter(([category]) => !manifest[category])
    .map(([category, where]) => `${category} (first used at ${where})`);
  assert.deepEqual(missing, [],
    `${manifestPath} must declare these required-reason API categories:\n  ${missing.join("\n  ")}`);
});

test("every declared category carries reasons Apple accepts from an application", () => {
  const manifest = declared(read(manifestPath));
  assert.notDeepEqual(Object.keys(manifest), []);
  for (const [category, reasons] of Object.entries(manifest)) {
    const family = families[category];
    assert.ok(family, `${category} is not a required-reason API category`);
    assert.ok(reasons.length > 0, `${category} declares no reason`);
    assert.equal(new Set(reasons).size, reasons.length, `${category} repeats a reason`);
    for (const reason of reasons) {
      assert.ok(family.reasons.includes(reason), `${reason} is not an approved reason for ${category}`);
    }
  }
});

test("the manifest is bundled with the application and the widget extension needs none", () => {
  const project = read(projectPath);
  assert.match(project, /- path: \.\.\/\.\.\/PrivacyInfo\.xcprivacy\s*\n\s*buildPhase: resources/,
    "project.yml no longer bundles PrivacyInfo.xcprivacy as an application resource");
  // The extension target lists only its own sources and the two shared
  // attribute models; it bundles no privacy manifest. That is only correct
  // while none of that code uses a required-reason API.
  for (const path of sharedWithExtension) {
    assert.ok(project.includes(path.replace("src-tauri/", "../../")),
      `${path} is no longer an extension source; update this test`);
  }
  const extension = used([...list(extensionSources, ".swift"), ...sharedWithExtension]);
  assert.deepEqual(extension, {},
    "The widget extension uses a required-reason API but bundles no privacy manifest of its own");
});
