#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Converts observed instrumentation JSON, never estimated benchmark results.
import fs from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';

const args = new Map();
for (let i = 2; i < process.argv.length; i += 2) {
  if (!process.argv[i].startsWith('--') || !process.argv[i + 1]) throw new Error('Expected --input DIR --manifest FILE --provenance FILE --output NEW_DIR');
  args.set(process.argv[i].slice(2), process.argv[i + 1]);
}
for (const key of ['input', 'manifest', 'provenance', 'output']) if (!args.has(key)) throw new Error(`Missing --${key}`);
const readJson = async file => JSON.parse(await fs.readFile(file, 'utf8'));
const manifestBytes = await fs.readFile(args.get('manifest'));
const manifest = JSON.parse(manifestBytes);
const manifestSha = createHash('sha256').update(manifestBytes).digest('hex');
const provenance = await readJson(args.get('provenance'));
if (manifest.schema !== 1 || !Array.isArray(manifest.cases)) throw new Error('Unsupported fixture manifest');
if (provenance.project !== 'kiwix-tauri-eval-jt' || provenance.billingEnabled !== false) throw new Error('Expected the approved, unbilled evaluation project');
for (const key of ['appApkSha256', 'testApkSha256', 'nativeSdkManifestSha256']) {
  if (!/^[a-f0-9]{64}$/.test(provenance[key] ?? '')) throw new Error(`Missing measured build identity: ${key}`);
}
if (typeof provenance.buildLabel !== 'string' || !provenance.buildLabel.trim()) throw new Error('Explicit optimization/instrumentation build label required');
const cases = new Map(manifest.cases.map(c => [c.id, c]));
const workloadSources = provenance.workloads ?? {};
for (const c of manifest.cases) {
  const source = workloadSources[c.id];
  if (!source || !['synthetic', 'production'].includes(source.origin) || !/^[a-f0-9]{64}$/.test(source.recipe?.sha256 ?? '') || !Array.isArray(source.contentLanguages) || !source.contentLanguages.length)
    throw new Error(`Missing exact recipe/content provenance: ${c.id}`);
}
async function files(dir) {
  const found = [];
  for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
    const file = path.join(dir, entry.name);
    if (entry.isSymbolicLink()) throw new Error(`Result symlink rejected: ${file}`);
    if (entry.isDirectory()) found.push(...await files(file));
    else if (entry.isFile() && entry.name.endsWith('.json')) found.push(file);
  }
  return found.sort();
}
const observations = [];
const auxiliary = [];
const records = [];
for (const file of await files(args.get('input'))) {
  const bytes = await fs.readFile(file);
  const value = JSON.parse(bytes);
  const source = path.relative(args.get('input'), file);
  if (value.schema !== 1) throw new Error(`Unsupported result schema: ${source}`);
  records.push({ value, source, directory: path.dirname(source), name: path.basename(source), sha256: createHash('sha256').update(bytes).digest('hex') });
}
if (!Array.isArray(provenance.executions) || !provenance.executions.length) throw new Error('Execution/submission linkage is required');
const executions = new Map();
for (const execution of provenance.executions) {
  if (!execution.id || !execution.runId || executions.has(execution.runId)) throw new Error('Missing or duplicate execution/run identity');
  if (!['firebase-physical', 'local-emulator'].includes(execution.kind)) throw new Error('Explicit physical or emulator execution kind required');
  const submission = execution.submission;
  if (!submission || submission.id !== execution.id || submission.project !== provenance.project || submission.manifestSha256 !== manifestSha)
    throw new Error(`Missing matching submission record: ${execution.runId}`);
  for (const key of ['appApkSha256', 'testApkSha256', 'nativeSdkManifestSha256'])
    if (submission[key] !== provenance[key]) throw new Error(`Submission build identity mismatch: ${execution.runId}/${key}`);
  if (submission.buildLabel !== provenance.buildLabel || !Array.isArray(submission.command) || !submission.command.length)
    throw new Error(`Submission build/command evidence missing: ${execution.runId}`);
  executions.set(execution.runId, execution);
}
const runs = new Map();
const variantKey = device => JSON.stringify([device.fingerprint, device.totalPhysicalRamBytes, device.supportedAbis, device.process64Bit, device.sdk, provenance.appApkSha256]);
const variants = new Map();
for (const record of records.filter(record => record.name === 'run.json')) {
  const run = record.value;
  const execution = executions.get(run.runId);
  if (!execution || execution.artifactDirectory !== record.directory || runs.has(record.directory)) throw new Error(`Unlinked or duplicate run: ${record.source}`);
  if (run.manifestSha256 !== manifestSha || run.benchmarkBuildLabel !== provenance.buildLabel || run.nativeSdkManifestSha256 !== provenance.nativeSdkManifestSha256)
    throw new Error(`Recorded run/build provenance conflict: ${record.source}`);
  if (!run.device?.fingerprint || !Number.isFinite(run.device.totalPhysicalRamBytes) || typeof run.device.process64Bit !== 'boolean')
    throw new Error(`Incomplete measured device identity: ${record.source}`);
  const key = variantKey(run.device);
  if (!variants.has(key)) variants.set(key, { id: `V${variants.size + 1}`, device: run.device });
  runs.set(record.directory, { runId: run.runId, execution, device: run.device, variant: variants.get(key).id, variantKey: key, observations: [], auxiliary: [], issues: [] });
}
if (!runs.size || runs.size !== executions.size) throw new Error('Missing retrieved run artifacts for a declared execution');
const kinds = new Set([...runs.values()].map(run => run.execution.kind));
if (kinds.size !== 1) throw new Error('Do not combine emulator smoke and physical-device measurements in one report');
const executionKind = [...kinds][0];
const seen = new Set();
for (const record of records) {
  const run = runs.get(record.directory);
  if (!run) throw new Error(`Artifact has no producing run: ${record.source}`);
  if (run.execution.artifacts?.[record.name] !== record.sha256) throw new Error(`Artifact digest/linkage mismatch: ${record.source}`);
  const value = record.value;
  if (value.device && variantKey(value.device) !== run.variantKey) throw new Error(`Device variant changed within run: ${record.source}`);
  const row = { ...value, source: record.source, runId: run.runId, executionId: run.execution.id, executionKind, variant: run.variant, variantKey: run.variantKey, device: value.device ?? run.device };
  if (value.caseId && value.mode) {
    if (!cases.has(value.caseId) || !['passed', 'failed'].includes(value.status)) throw new Error(`Invalid workload observation: ${record.source}`);
    if (value.native?.manifestSha256 && value.native.manifestSha256 !== manifestSha) throw new Error(`Stale manifest result: ${record.source}`);
    if (value.native?.recipeSha256 && value.native.recipeSha256 !== workloadSources[value.caseId].recipe.sha256)
      throw new Error(`Measured recipe differs from workload provenance: ${record.source}`);
    if (value.status === 'passed' && value.native?.ok !== true) throw new Error(`Successful row has no native success: ${record.source}`);
    if (value.status === 'passed' && ['cold', 'warm', 'collision', 'boundaries'].includes(value.mode)) {
      const validity = value.native?.validity;
      const expected = cases.get(value.caseId).expected;
      if (!validity || validity.sha256 !== expected.sha256 || validity.size !== expected.size || validity.metadata?.id !== expected.uuid || validity.retainedChecksum !== true || validity.structural !== true || !/^[a-f0-9]{32}$/.test(validity.zimFooterMd5 ?? ''))
        throw new Error(`Successful output lacks matching integrity evidence: ${record.source}`);
    }
    const key = JSON.stringify([run.runId, value.caseId, value.mode, value.repetition]);
    if (seen.has(key)) throw new Error(`Duplicate observation would inflate sample count: ${record.source}`);
    seen.add(key);
    observations.push(row);
    run.observations.push(row);
  } else {
    auxiliary.push(row);
    run.auxiliary.push(row);
  }
}
for (const [directory, run] of runs) {
  const retrieved = records.filter(record => record.directory === directory).map(record => record.name).sort();
  if (JSON.stringify(retrieved) !== JSON.stringify(Object.keys(run.execution.artifacts ?? {}).sort())) throw new Error(`Incomplete artifact retrieval: ${run.runId}`);
  const summaries = run.auxiliary.filter(row => path.basename(row.source) === 'summary.json');
  const setupFailures = run.auxiliary.filter(row => row.caseId && row.status === 'failed').length;
  const observedFailures = run.observations.filter(row => row.status === 'failed').length + setupFailures;
  run.recordedFailures = observedFailures;
  if (summaries.length !== 1) run.issues.push('Final producer summary missing or duplicated; run completeness unknown');
  else {
    const summary = summaries[0];
    if (!Number.isInteger(summary.executed) || !Number.isInteger(summary.failures) || summary.executed < 0 || summary.failures < 0) throw new Error(`Invalid producer summary: ${run.runId}`);
    run.summary = summary;
    run.recordedFailures = Math.max(summary.failures, observedFailures);
    if (summary.executed !== run.observations.length) run.issues.push(`Producer executed ${summary.executed}; retrieved ${run.observations.length} workload observations`);
    if (summary.failures !== observedFailures) run.issues.push(`Producer failures ${summary.failures}; retrieved ${observedFailures} failure records`);
  }
}
// Old schema-1 producers identify their preseeded policy in cachePolicy.
const workloadPolicy = row => {
  const explicit = row.native?.workloadPolicy;
  if (explicit !== undefined) {
    if (!['preseeded-map-cache-v1', 'bounded-map-cache-raw-ranges-v2'].includes(explicit))
      throw new Error(`Unsupported workload policy: ${JSON.stringify(explicit)}`);
    return explicit;
  }
  return row.native?.cachePolicy?.includes('staged-before-timing') ? 'preseeded-map-cache-v1' : 'unreported';
};
const measuredPolicies = new Set(observations.filter(row => row.native?.composition).map(workloadPolicy));
if (measuredPolicies.size > 1) throw new Error('Do not combine different map cache/workload policies in one report');
if (measuredPolicies.has('unreported')) throw new Error('Measured composition has no identifiable workload policy');
const numeric = value => typeof value === 'number' && Number.isFinite(value) ? value : null;
const delta = (metrics, key) => {
  const before = numeric(metrics.ioBefore?.[key]);
  const after = numeric(metrics.ioAfter?.[key]);
  return before !== null && after !== null && after >= before ? after - before : null;
};
const mib = value => value === null ? null : value / 1048576;
const number = (value, digits = 2) => numeric(value) === null ? '—' : value.toFixed(digits);
const escape = value => String(value ?? '—').replaceAll('|', '\\|').replaceAll('\n', ' ');
const phaseIds = { composition: '2', identitySha256: '3', retainedChecksum: '4', structuralValidation: '5', combinedIntegrity: '6' };
const measurements = [];
for (const row of observations) {
  const native = row.native ?? {};
  const phases = Object.entries(phaseIds).map(([name, id]) => [name, native[name], id]);
  if (native.composition?.chunkSha256) phases.push(['chunkSha256', native.composition.chunkSha256, '2']);
  for (const [phase, metrics, id] of phases) {
    if (!metrics) continue;
    const memory = row.phaseMemory?.[id];
    const peak = memory?.samples > 0 ? memory.peak ?? {} : {};
    const elapsedUs = numeric(metrics.elapsedUs);
    const bytes = numeric(metrics.bytes);
    const isPerformance = row.status === 'passed' && ['cold', 'warm'].includes(row.mode);
    measurements.push({
      device: row.device?.device ?? row.device?.model ?? 'unreported', model: row.device?.model ?? null,
      sdk: row.device?.sdk ?? null, totalPhysicalRamBytes: numeric(row.device?.totalPhysicalRamBytes),
      runId: row.runId, executionId: row.executionId, executionKind: row.executionKind, variant: row.variant, variantKey: row.variantKey,
      caseId: row.caseId, tier: row.tier, mode: row.mode, repetition: row.repetition, status: row.status, phase,
      workloadPolicy: workloadPolicy(row),
      bytes, elapsedUs, cpuUs: numeric(phase === 'chunkSha256' ? metrics.threadCpuUs : metrics.processCpuUs),
      cpuKind: phase === 'chunkSha256' ? 'thread' : 'process',
      outputOrDigestMiBPerSecond: isPerformance && ['composition', 'identitySha256', 'retainedChecksum', 'chunkSha256'].includes(phase) && elapsedUs > 0 && bytes !== null ? mib(bytes) * 1000000 / elapsedUs : null,
      baselineRssBytes: numeric(row.baseline?.rssBytes), baselinePssBytes: numeric(row.baseline?.pssBytes),
      peakRssBytes: numeric(peak.rssBytes), peakPssBytes: numeric(peak.pssBytes),
      nativePssBytes: numeric(peak.nativePssBytes), dalvikPssBytes: numeric(peak.dalvikPssBytes),
      nativeHeapAllocatedBytes: numeric(peak.nativeHeapAllocatedBytes), jvmUsedHeapBytes: numeric(peak.jvmUsedHeapBytes),
      samples: memory?.samples ?? 0, samplingPeriodMs: row.samplingPeriodMs ?? null,
      memoryScope: phase === 'chunkSha256' ? 'entire-composition-not-digest-isolated' : phase,
      logicalReadBytes: delta(metrics, 'rchar'), logicalWriteBytes: delta(metrics, 'wchar'),
      physicalReadBytes: delta(metrics, 'read_bytes'), physicalWriteBytes: delta(metrics, 'write_bytes'),
      minimumAvailableStorageBytes: numeric(memory?.minimumAvailableStorageBytes),
      source: row.source,
    });
  }
}
const groups = new Map();
for (const row of measurements.filter(r => r.status === 'passed' && ['cold', 'warm'].includes(r.mode))) {
  const key = JSON.stringify([row.variantKey, row.caseId, row.mode, row.phase, row.workloadPolicy]);
  if (!groups.has(key)) groups.set(key, []);
  groups.get(key).push(row);
}
const median = values => {
  const sorted = values.filter(v => numeric(v) !== null).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
function table(headers, rows) {
  return [headers, headers.map(() => '---'), ...rows].map(row => '| ' + row.map(escape).join(' | ') + ' |').join('\n');
}
const summaries = [...groups.values()].map(rows => ({
  ...rows[0], observations: rows.length,
  elapsedCount: rows.filter(r => r.elapsedUs !== null).length,
  cpuCount: rows.filter(r => r.cpuUs !== null).length,
  throughputCount: rows.filter(r => r.outputOrDigestMiBPerSecond !== null).length,
  memoryCount: rows.filter(r => r.peakPssBytes !== null).length,
  minimumBytes: Math.min(...rows.filter(r => r.bytes !== null).map(r => r.bytes)),
  maximumBytes: Math.max(...rows.filter(r => r.bytes !== null).map(r => r.bytes)),
  medianElapsedMs: median(rows.map(r => r.elapsedUs === null ? null : r.elapsedUs / 1000)),
  minElapsedMs: Math.min(...rows.filter(r => r.elapsedUs !== null).map(r => r.elapsedUs / 1000)),
  maxElapsedMs: Math.max(...rows.filter(r => r.elapsedUs !== null).map(r => r.elapsedUs / 1000)),
  medianCpuMs: median(rows.map(r => r.cpuUs === null ? null : r.cpuUs / 1000)),
  medianThroughput: median(rows.map(r => r.outputOrDigestMiBPerSecond)),
  maximumSampledPssMiB: (() => { const values = rows.map(r => mib(r.peakPssBytes)).filter(v => v !== null); return values.length ? Math.max(...values) : null; })(),
}));
const devices = [...variants.values()];
const assembly = summaries.filter(r => r.phase === 'composition');
const checksums = summaries.filter(r => ['chunkSha256', 'retainedChecksum', 'identitySha256'].includes(r.phase));
const failures = observations.filter(r => r.status !== 'passed');
const nonWorkloadFailures = auxiliary.filter(r => r.caseId && ['failed', 'skipped'].includes(r.status));
const negatives = observations.filter(r => !['cold', 'warm'].includes(r.mode));
const sourceKinds = provenance.workloads ?? {};
const recordedFailureCount = [...runs.values()].reduce((total, run) => total + run.recordedFailures, 0);
const incompleteRuns = [...runs.values()].filter(run => run.issues.length);
const verifiedOutputs = new Map();
for (const row of observations.filter(row => row.status === 'passed' && ['cold', 'warm'].includes(row.mode))) {
  const validity = row.native.validity;
  const key = JSON.stringify([row.caseId, validity.sha256, validity.zimFooterMd5]);
  if (!verifiedOutputs.has(key)) verifiedOutputs.set(key, { caseId: row.caseId, validity, variants: new Set() });
  verifiedOutputs.get(key).variants.add(row.variant);
}
const report = [
  executionKind === 'firebase-physical' ? '# Low-end Android ZIM assembly and checksum evaluation' : '# Android emulator smoke — not physical-device performance', '',
  `Experiment: ${escape(provenance.experimentDate ?? 'date not supplied')}. ${executionKind}.`, '',
  `**Observed:** ${devices.length} measured device variants, ${observations.filter(r => r.status === 'passed').length} successful workload/assertion observations, ${recordedFailureCount} recorded failures, ${nonWorkloadFailures.filter(r => r.status === 'skipped').length} skipped cases, **${incompleteRuns.length} incomplete/inconsistent runs**. Success applies only to the listed workloads, sizes and variants; missing evidence is not a pass.`, '',
  '## Build and experiment identity', '',
  `- Project: \`${provenance.project}\`; billing disabled. No paid-test authorization.`,
  `- Build: ${escape(provenance.buildLabel)}.`,
  `- Application APK SHA-256: \`${provenance.appApkSha256}\`.`,
  `- Instrumentation APK SHA-256: \`${provenance.testApkSha256}\`.`,
  `- Native SDK manifest SHA-256: \`${provenance.nativeSdkManifestSha256}\`.`,
  `- Fixture manifest SHA-256: \`${manifestSha}\`.`,
  `- Workload policy: \`${[...measuredPolicies][0] ?? 'no measured composition'}\`; different policies are never pooled.`,
  `- Execution references: ${[...executions.values()].map(execution => escape(execution.id)).join(', ')}.`, '',
  '## Producing runs and retrieval completeness', '',
  table(['Run / execution', 'Retrieved / executed', 'Recorded failures', 'Coverage issues'], [...runs.values()].map(run => [
    `${run.runId} / ${run.execution.id}`, `${run.observations.length} / ${run.summary?.executed ?? 'unknown'}`, run.recordedFailures, run.issues.join('; ') || 'Counts reconciled',
  ])), '',
  '## Devices actually observed', '',
  table(['Variant', 'Model / device', 'Android / API', 'Reported RAM (GiB)', '64-bit process', 'SoC'], devices.map(({ id, device: d }) => [
    id, `${d.model} / ${d.device}`, `${d.osRelease} / ${d.sdk}`, number(d.totalPhysicalRamBytes / 1073741824), d.process64Bit, `${d.socManufacturer ?? d.hardware} ${d.socModel ?? ''}`,
  ])), '',
  '## Exact workload', '',
  'The measured implementation is the app’s production recipe-based StreetZim assembler, including verified source reuse, chunk SHA-256, generated ZIM MD5, synchronization and protected publication. It is not an arbitrary-ZIM merge implementation. Independent verification scans below are additional measurements, not extra assembly stages to sum into a single-pass total.', '',
  table(['Case', 'Origin', 'Output MiB', 'Recipe KiB / segments', 'Primary language tag', 'Content languages', 'Page / search checks'], manifest.cases.filter(c => observations.some(r => r.caseId === c.id) || nonWorkloadFailures.some(r => r.caseId === c.id)).map(c => [
    c.id, sourceKinds[c.id].origin, number(c.expected.size / 1048576), `${number(sourceKinds[c.id].recipe.size / 1024)} / ${sourceKinds[c.id].recipeSegmentCount}`, c.expected.languages.join(';'), sourceKinds[c.id].contentLanguages.join(';'), `${c.expected.articles.length} / ${c.expected.searches.length}`,
  ])), '',
  'Workload source URLs, input/recipe identities and detailed producer characteristics are preserved in `provenance.json`, `fixture-manifest.json` and `observations.json`. Metadata language labels are not proof of the languages actually present; article and search assertions exercise content separately.', '',
  '## Assembly performance', '',
  'Times are milliseconds; throughput is output MiB/s (1 MiB = 1,048,576 bytes). With one observation the displayed central value is that single observation, not an estimate of population performance. Memory is the maximum sampled process PSS during composition, including instrumentation/JVM/native runtime; it is not a continuous high-water mark.', '',
  table(['Variant', 'Case / mode', 'Runs / timed', 'Time central [min–max] ms', 'CPU ms [available]', 'MiB/s [available]', 'Sampled PSS MiB [available]'], assembly.map(r => [
    r.variant, `${r.caseId} / ${r.mode}`, `${r.observations} / ${r.elapsedCount}`, `${number(r.medianElapsedMs)} [${number(r.minElapsedMs)}–${number(r.maxElapsedMs)}]`, `${number(r.medianCpuMs)} [${r.cpuCount}/${r.observations}]`, `${number(r.medianThroughput)} [${r.throughputCount}/${r.observations}]`, `${number(r.maximumSampledPssMiB)} [${r.memoryCount}/${r.observations}]`,
  ])), '',
  '## Checksum performance', '',
  'Chunk SHA-256 timings are nested inside assembly and count actual attempted chunk bytes, including rejected candidates. Its CPU clock is thread time; other checksum CPU rows use process time. Retained ZIM MD5 digests output bytes minus the 16-byte footer; independent whole-file SHA-256 digests the complete output. These are different operations, not interchangeable throughput claims.', '',
  table(['Variant', 'Case / mode', 'Operation', 'Runs / timed', 'Digest bytes [min–max]', 'Time central ms', 'CPU ms [available]', 'MiB/s [available]'], checksums.map(r => [
    r.variant, `${r.caseId} / ${r.mode}`, r.phase, `${r.observations} / ${r.elapsedCount}`, `${number(r.minimumBytes, 0)}–${number(r.maximumBytes, 0)}`, number(r.medianElapsedMs), `${number(r.medianCpuMs)} [${r.cpuCount}/${r.observations}]`, `${number(r.medianThroughput)} [${r.throughputCount}/${r.observations}]`,
  ])), '',
  '## Validity and negative cases', '',
  table(['Case', 'Validated variants', 'Output bytes', 'Whole-file SHA-256', 'ZIM footer MD5'], [...verifiedOutputs.values()].map(output => [
    output.caseId, [...output.variants].join(', '), output.validity.size, output.validity.sha256, output.validity.zimFooterMd5,
  ])), '',
  table(['Variant / run', 'Case / mode', 'Result', 'Evidence / error'], [...negatives, ...failures.filter(r => ['cold', 'warm'].includes(r.mode)), ...nonWorkloadFailures].map(r => [
    `${r.variant} / ${r.runId}`, `${r.caseId} / ${r.mode ?? 'setup'}`, r.status, r.native?.error ?? r.reason ?? JSON.stringify(r.native?.assertions ?? { collisionPreserved: r.native?.collisionPreserved, validity: r.native?.validity }),
  ])), '',
  '## Interpretation and limits', '',
  '- Cold/warm describe the recorded workload policy, not controlled OS caches. Under preseeded-map-cache-v1, verified source/pack cache preparation is outside assembly timing. Under bounded-map-cache-raw-ranges-v2, cold starts with an empty 8 MiB cache and warm prewarms a deterministic prefix up to that budget; retained read-only source ranges enter the common mask/SHA/cache/composition path on misses inside timing. These policies are not interchangeable measurements.',
  '- Sampled peaks are lower bounds; a phase with no sample is unavailable, never zero memory. Baselines, RSS, PSS, native/Dalvik PSS, allocated heaps, sample counts and intervals are in `phases.csv`. Process CPU includes the instrumentation sampler.',
  '- The runner invokes production Rust/native archive work inside the instrumented target process; it does not launch the interactive reader WebView. Process memory is therefore workload-plus-instrumentation memory, not the complete interactive application’s memory footprint.',
  '- `/proc/self/io` logical counters are not physical disk traffic. Unavailable counters remain blank; natural storage insufficiency is reported as a skip, not successful large-file support. No whole-device disk-filling experiment is implied.',
  '- No percentile or long-term reliability claim is made from this small experiment. Device families have multiple RAM/storage/ABI variants; only actual observed variants are covered.',
  '- ARM64-only results do not establish support for 32-bit Android, absent catalog models, larger real recipes, or arbitrary archive combinations. Emulator smoke and optimized physical-device performance must not be mixed.',
  '- Public fixtures only. See raw `provenance.json` for exact build flags, sources, execution references, limitations and reproduction commands.', '',
].join('\n');
const columns = Object.keys(measurements[0] ?? {});
const csvCell = value => value === null || value === undefined ? '' : '"' + String(value).replaceAll('"', '""') + '"';
const csv = [columns.join(','), ...measurements.map(row => columns.map(c => csvCell(row[c])).join(','))].join('\n') + '\n';
const output = path.resolve(args.get('output'));
await fs.mkdir(output); // Exclusive report directory: do not overwrite an unrelated report.
await Promise.all([
  fs.writeFile(path.join(output, 'README.md'), report),
  fs.writeFile(path.join(output, 'phases.csv'), csv),
  fs.writeFile(path.join(output, 'observations.json'), JSON.stringify({ observations, auxiliary }, null, 2) + '\n'),
  fs.writeFile(path.join(output, 'provenance.json'), JSON.stringify(provenance, null, 2) + '\n'),
  fs.writeFile(path.join(output, 'fixture-manifest.json'), manifestBytes),
]);
console.log(JSON.stringify({ output, devices: devices.length, observations: observations.length, failures: recordedFailureCount, incompleteRuns: incompleteRuns.length, skipped: nonWorkloadFailures.filter(r => r.status === 'skipped').length, manifestSha256: manifestSha }, null, 2));
