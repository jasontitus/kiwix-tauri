import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const shell = await readFile(new URL("../src/styles.css", import.meta.url), "utf8");
const platform = await readFile(new URL("../src/platform.css", import.meta.url), "utf8");
const archives = await readFile(new URL("../src/components/library-catalog.css", import.meta.url), "utf8");
const reader = await readFile(new URL("../src/components/reader.css", import.meta.url), "utf8");

// Source-level palette checks, not a browser cascade/layout or accessibility
// certification. Read the shipped tokens so changing a color cannot silently
// regress the repaired normal-size action/navigation labels.
function declarations(css, selector, required = true) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const block = css.match(new RegExp(`(?:^|\\n)\\s*${escaped}\\s*\\{([^{}]*)\\}`));
  if (!block && !required) return {};
  assert.ok(block, `Missing rule: ${selector}`);
  return Object.fromEntries([...block[1].matchAll(/([\w-]+)\s*:\s*([^;]+);/g)]
    .map(([, property, value]) => [property, value.trim()]));
}

function palette(name, theme) {
  const selector = `:root[data-platform="${name}"]`;
  return {
    ...declarations(shell, ":root"),
    ...declarations(platform, selector),
    ...(theme === "light" ? {} : declarations(platform, `${selector}[data-theme="${theme}"]`)),
  };
}

function color(tokens, value, visited = new Set()) {
  const reference = value.match(/^var\((--[\w-]+)\)$/);
  if (reference) {
    const key = reference[1];
    assert.ok(!visited.has(key), `Cyclic color token ${key}`);
    assert.ok(tokens[key], `Undefined color token ${key}`);
    return color(tokens, tokens[key], new Set([...visited, key]));
  }
  const mix = value.match(/^color-mix\(in srgb, (var\(--[\w-]+\)), (#[\da-f]{6}) (\d+)%\)$/i);
  if (mix) {
    const a = color(tokens, mix[1], visited), b = color(tokens, mix[2], visited);
    const amount = Number(mix[3]) / 100;
    return a.slice(0, 3).map((channel, index) => channel * (1 - amount) + b[index] * amount);
  }
  assert.match(value, /^#[\da-f]{6}(?:[\da-f]{2})?$/i, `Unsupported palette color ${value}`);
  return [1, 3, 5, ...(value.length === 9 ? [7] : [])]
    .map((offset) => parseInt(value.slice(offset, offset + 2), 16) / 255);
}

function luminance(rgb) {
  return rgb.slice(0, 3).map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4)
    .reduce((total, value, i) => total + value * [0.2126, 0.7152, 0.0722][i], 0);
}

function contrast(foreground, background) {
  const alpha = foreground[3] ?? 1;
  const composite = foreground.slice(0, 3).map((value, i) => alpha * value + (1 - alpha) * background[i]);
  const a = luminance(composite), b = luminance(background);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

function expectContrast(tokens, foreground, background, context) {
  const ratio = contrast(color(tokens, foreground), color(tokens, background));
  assert.ok(ratio >= 4.5, `${context}: ${ratio.toFixed(3)}:1 is below 4.5:1`);
}

function expectControlContrast(tokens, properties, context) {
  const opacity = Number(properties.opacity ?? 1);
  for (const backdrop of ["--paper", "--surface"]) {
    const parent = color(tokens, `var(${backdrop})`);
    const composite = (value) => color(tokens, value).slice(0, 3)
      .map((channel, index) => channel * opacity + parent[index] * (1 - opacity));
    const ratio = contrast(composite(properties.color), composite(properties.background));
    assert.ok(ratio >= 4.5, `${context}/${backdrop} at opacity ${opacity}: ${ratio.toFixed(3)}:1 is below 4.5:1`);
  }
}

for (const name of ["ios", "macos", "linux"]) {
  test(`${name} normal-size primary labels retain contrast in light, dark and system-dark palettes`, () => {
    for (const theme of ["light", "dark", "system"]) {
      const tokens = palette(name, theme);
      const selector = `:where(:root[data-platform="${name}"]) button.primary`;
      const normal = { ...declarations(shell, "button.primary"), ...declarations(platform, selector) };
      const hover = {
        ...normal,
        ...declarations(shell, "button.primary:hover:not(:disabled)"),
        ...declarations(platform, `${selector}:hover:not(:disabled)`, false),
      };
      const pressed = {
        ...normal,
        ...declarations(platform, `:where(:root[data-platform="${name}"]) button:active:not(:disabled)`, false),
        ...declarations(platform, `${selector}:active:not(:disabled)`),
      };
      for (const [state, properties] of Object.entries({ normal, hover, pressed }))
        expectControlContrast(tokens, properties, `${name}/${theme}/${state}`);
    }
  });
}

test("reader filled actions keep contrast through platform pressed styles", () => {
  for (const name of ["ios", "macos", "linux", "android", "windows"]) {
    for (const theme of ["light", "dark", "system"]) {
      const tokens = palette(name, theme);
      for (const selector of [".reader-workspace .rw-primary", ".rw-search-input > button"]) {
        const normal = declarations(reader, selector);
        const hover = { ...normal, ...declarations(reader, `${selector}:hover:not(:disabled)`) };
        const pressed = {
          ...normal,
          ...declarations(platform, `:where(:root[data-platform="${name}"]) button:active:not(:disabled)`, false),
          ...declarations(reader, `${selector}:active:not(:disabled)`),
        };
        for (const [state, properties] of Object.entries({ normal, hover, pressed }))
          expectControlContrast(tokens, properties, `${name}/${theme}/${selector}/${state}`);
      }
    }
  }
});

test("Linux secondary text remains readable on shell, card and navigation surfaces", () => {
  for (const theme of ["light", "dark", "system"]) {
    const tokens = palette("linux", theme);
    for (const surface of ["--paper", "--surface", "--sidebar"])
      expectContrast(tokens, "var(--muted)", `var(${surface})`, `linux/${theme}/${surface}`);
  }
});

test("iOS inactive destinations remain readable on the tab bar's light and dark surfaces", () => {
  for (const theme of ["light", "dark", "system"]) {
    const tokens = palette("ios", theme);
    for (const surface of ["--paper", "--surface"])
      expectContrast(tokens, "var(--tab-inactive)", `var(${surface})`, `ios/${theme}/${surface}`);
  }
});

test("iOS filled tablet navigation retains contrast when pressed", () => {
  const selector = ':where(:root[data-platform="ios"]) .sidebar-settings.active';
  const normal = declarations(platform, selector);
  const pressed = {
    ...normal,
    ...declarations(platform, ':where(:root[data-platform="ios"]) button:active:not(:disabled)'),
    ...declarations(platform, `${selector}:active:not(:disabled)`),
  };
  for (const theme of ["light", "dark", "system"]) {
    const tokens = palette("ios", theme);
    for (const [state, properties] of Object.entries({ normal, pressed }))
      expectControlContrast(tokens, properties, `ios/${theme}/tablet-navigation/${state}`);
  }
});

test("filled actions consume separate foreground and background tokens", () => {
  const primary = declarations(shell, "button.primary");
  assert.equal(primary.background, "var(--primary-background)");
  assert.equal(primary.color, "var(--primary-foreground)");
});

test("archive content typography participates in the native text-size contract", () => {
  const sizes = [...archives.matchAll(/font-size:\s*([^;]+);/g)];
  assert.ok(sizes.length > 0);
  for (const [, size] of sizes)
    assert.match(size, /^calc\(\d+px \* var\(--system-text-scale, 1\)\)$/, `Unscaled archive text: ${size}`);
});

test("download emphasis uses the defined foreground token", () => {
  assert.equal(declarations(shell, ".download-live-note").color, "var(--ink)");
  assert.equal(declarations(shell, ".share-magnet").color, "var(--ink)");
});
