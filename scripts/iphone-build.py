#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build exact committed iPhone apps off-phone; never contact a device.

Prepare: python3 scripts/iphone-build.py --prepare-only --signing-source /path/Kiwix.app
Build:   python3 scripts/iphone-build.py --build --signing-source /path/Kiwix.app
Sign:    python3 scripts/iphone-build.py --sign --signing-source /path/Kiwix.app
Upstream: add --product kiwix-apple --allow-payment-entitlement-omission.
Simulator: add --platform simulator; no signing source or physical credentials.
Simulator receipts bind arm64 Mach-O IOSSIMULATOR inspection to the full app inventory.
The upstream backend may fetch its pinned release, published framework and SPM
dependencies. It is a source-built comparison, not an App Store binary.
The default action builds and signs. Independent review is a workflow gate,
not an assertion made by this tool. JSON stdout names the resulting manifest.
"""
import argparse
import datetime as dt
import fcntl
import fnmatch
import hashlib
import json
import os
import re
from pathlib import Path, PurePosixPath
import plistlib
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import time


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def object_hash(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def save(path, value):
    path = Path(path)
    require(not path.is_symlink(), f'Refusing symlink output: {path}')
    temporary = path.with_name(path.name + '.new')
    with temporary.open('x') as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write('\n')
    os.replace(temporary, path)


def tree(path):
    """Canonical relative-name/content/executable-mode list; never follow links."""
    path = Path(path)
    entries = []
    for base, dirs, files in os.walk(path, followlinks=False):
        for name in sorted(dirs + files):
            child = Path(base) / name
            info = child.lstat()
            rel = child.relative_to(path).as_posix()
            if stat.S_ISLNK(info.st_mode):
                target = os.readlink(child)
                require(child.resolve().is_relative_to(path.resolve()), f'Escaping bundle link: {child}')
                entries.append([rel, 'link', target])
            elif stat.S_ISREG(info.st_mode):
                entries.append([rel, 'file', digest(child), info.st_mode & 0o111])
            else:
                require(stat.S_ISDIR(info.st_mode), f'Unsupported file: {child}')
    return sorted(entries)


class Build:
    def __init__(self, args):
        self.args = args
        self.simulator = args.platform == 'simulator'
        self.target = 'aarch64-apple-ios-sim' if self.simulator else 'aarch64-apple-ios'
        self.apple_sdk = 'iphonesimulator' if self.simulator else 'iphoneos'
        require(not self.simulator or not any((args.signing_source, args.signing_identity, args.device_udid)),
                'Simulator builds must not receive physical signing inputs or device identifiers')
        self.repo = Path(args.repo).expanduser().resolve(strict=True)
        self.root = Path(args.output or self.repo / '.native/iphone-tool').expanduser().absolute()
        # No output through a symlink, even if its destination would be writable.
        for part in [self.root, *self.root.parents]:
            require(not part.is_symlink(), f'Symlink output ancestor: {part}; use its canonical path')
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        require(self.root.stat().st_uid == os.getuid(), 'Output is not owned by this user')
        os.chmod(self.root, 0o700)
        marker = self.root / 'owner.json'
        identity = {'schema': 1, 'repo': str(self.repo), 'tool': 'kiwix-iphone-build'}
        if marker.exists():
            require(json.loads(marker.read_text()) == identity, 'Output belongs to another recipe/repository')
        else:
            require(not any(self.root.iterdir()), 'Refusing unowned nonempty output')
            save(marker, identity)
        self.lock = (self.root / 'build.lock').open('a')
        fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        self.deadline = time.monotonic() + args.timeout
        self.logs = self.root / 'logs'
        self.logs.mkdir(exist_ok=True, mode=0o700)
        self.counter = 0
        self.commands = []
        self.sdk = Path(args.sdk or self.repo / '.native/mobile-retained-fd-v2' / self.target).expanduser().resolve(strict=True) if args.product == 'tauri' else None
        self.env = {key: os.environ[key] for key in ('HOME', 'USER', 'LOGNAME', 'TMPDIR', 'SSH_AUTH_SOCK', 'DEVELOPER_DIR') if key in os.environ}
        # Discover installed tools, but do not inherit compiler/SDK/flag overrides.
        rust = Path.home() / '.rustup/toolchains/stable-aarch64-apple-darwin/bin'
        node = shutil.which('node')
        require(node or args.product == 'kiwix-apple', 'Node must already be installed and on PATH; no installation is attempted')
        xcode_wrapper = os.environ.get('KIWIX_XCODE_WRAPPER')
        if xcode_wrapper:
            xcode_wrapper = Path(xcode_wrapper).expanduser().resolve()
            require(xcode_wrapper.is_dir(), 'KIWIX_XCODE_WRAPPER must be an existing directory')
            for tool in ('xcodebuild', 'xcrun'):
                require((xcode_wrapper / tool).is_file(), f'KIWIX_XCODE_WRAPPER lacks {tool}')
        self.xcode_wrapper = xcode_wrapper
        path_parts = [str(rust)]
        if node:
            path_parts.append(str(Path(node).resolve().parent))
        if xcode_wrapper:
            path_parts.append(str(xcode_wrapper))
        path_parts += ['/opt/homebrew/bin', '/usr/bin', '/bin', '/usr/sbin', '/sbin']
        self.env.update(PATH=os.pathsep.join(path_parts),
                        CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS=os.environ.get('KIWIX_CARGO_JOBS', '4'), CARGO_TARGET_DIR=str(self.root / 'cargo-target'),
                        RUSTC=str(rust / 'rustc'),
                        CI='true', npm_config_offline='true', npm_config_audit='false', npm_config_fund='false',
                        TAURI_ENV_PLATFORM='ios', CARGO_INCREMENTAL=os.environ.get('KIWIX_CARGO_INCREMENTAL', '0'))
        if self.sdk:
            self.env['TAURI_KIWIX_MOBILE_SDK_ROOT'] = str(self.sdk)
        if os.environ.get('KIWIX_CODEGEN_UNITS'):
            self.env['CARGO_PROFILE_RELEASE_CODEGEN_UNITS'] = os.environ['KIWIX_CODEGEN_UNITS']

    def run(self, command, cwd=None, merged=False):
        self.counter += 1
        log = self.logs / f'{time.time_ns()}-{self.counter:03d}-{Path(command[0]).name}.log'
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, 'Offline preparation/build deadline expired')
        print('iphone-build: ' + ' '.join(command) + f' [logs: {log}]', file=sys.stderr, flush=True)
        error_log = log.with_suffix('.stderr.log')
        self.commands.append({'argv': command, 'cwd': str(cwd or self.repo), 'log': str(log), 'stderr_log': str(error_log)})
        with log.open('xb') as output, error_log.open('xb') as errors:
            process = subprocess.Popen(command, cwd=cwd or self.repo, env=self.env, stdin=subprocess.DEVNULL,
                                       stdout=output, stderr=subprocess.STDOUT if merged else errors, start_new_session=True)
            try:
                while True:
                    remaining = self.deadline - time.monotonic()
                    require(remaining > 0, 'Offline preparation/build deadline expired')
                    try:
                        code = process.wait(timeout=min(15, remaining))
                        break
                    except subprocess.TimeoutExpired:
                        print(f'iphone-build: still running {command[0]}; output remains in {log} and {error_log}', file=sys.stderr, flush=True)
            except BaseException:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                raise
        require(code == 0, f'Command failed ({code}): {command}; log: {log}')
        return log.read_bytes()

    def capture(self, command, cwd=None, merged=False):
        return self.run(command, cwd, merged=merged).decode().strip()

    def wrapper_tooling(self):
        if not self.xcode_wrapper:
            return {}
        return {'xcode_wrapper': str(self.xcode_wrapper),
                'xcode_wrapper_xcodebuild': self.capture([str(self.xcode_wrapper / 'xcodebuild'), '-version']),
                'xcode_wrapper_sdk': self.capture([str(self.xcode_wrapper / 'xcrun'), '--sdk',
                    self.apple_sdk, '--show-sdk-version'])}

    def profile(self, bundle, destination):
        path = bundle / 'embedded.mobileprovision'
        require(path.is_file() and not path.is_symlink(), f'Missing regular profile: {path}')
        data = self.run(['/usr/bin/security', 'cms', '-D', '-i', str(path)])
        # security cms emits the decoded plist to stdout on success.
        profile = plistlib.loads(data)
        now = dt.datetime.now(dt.timezone.utc).replace(tzinfo=None)
        require(profile['CreationDate'] <= now < profile['ExpirationDate'], 'Provisioning profile not currently valid')
        require('iOS' in profile['Platform'], 'Profile is not for iOS')
        require(not self.args.device_udid or self.args.device_udid in profile.get('ProvisionedDevices', []), 'Requested device absent from profile')
        # Extract the XML plist separately from codesign diagnostics.
        raw = self.run(['/usr/bin/codesign', '-d', '--entitlements', ':-', str(bundle)])
        start, end = raw.find(b'<?xml'), raw.find(b'</plist>')
        require(start >= 0 and end >= start, 'Cannot extract signed entitlements')
        ent = plistlib.loads(raw[start:end + len(b'</plist>')])
        info = plistlib.loads((bundle / 'Info.plist').read_bytes())
        team = ent['com.apple.developer.team-identifier']
        require(profile['TeamIdentifier'] == [team], 'Entitlement/profile team mismatch')
        require(ent['application-identifier'] == team + '.' + info['CFBundleIdentifier'], 'Application identifier mismatch')
        for key, value in ent.items():
            grant = profile['Entitlements'].get(key)
            if isinstance(value, str) and isinstance(grant, str):
                require(fnmatch.fnmatchcase(value, grant), f'Capability not granted: {key}')
            else:
                require(value == grant, f'Capability not granted exactly: {key}')
        destination.mkdir(mode=0o700, exist_ok=True)
        for target in (destination / 'embedded.mobileprovision', destination / 'entitlements.plist'):
            require(not target.is_symlink(), f'Symlink signing input: {target}')
        shutil.copyfile(path, destination / 'embedded.mobileprovision')
        (destination / 'entitlements.plist').write_bytes(plistlib.dumps(ent))
        certs = profile['DeveloperCertificates']
        return {'bundle_id': info['CFBundleIdentifier'], 'team': team, 'profile_uuid': profile['UUID'],
                'profile_expiration': profile['ExpirationDate'].isoformat() + 'Z',
                'provisioned_devices': profile.get('ProvisionedDevices', []), 'profile_sha256': digest(path),
                'entitlements_sha256': digest(destination / 'entitlements.plist'), 'entitlements': ent,
                'certificate_sha1s': [hashlib.sha1(cert).hexdigest().upper() for cert in certs]}

    def inputs(self):
        self.commit = self.capture(['/usr/bin/git', 'rev-parse', '--verify', self.args.revision + '^{commit}'])
        sdk_manifest = self.sdk / 'kiwix-sdk.json'
        sdk = json.loads(sdk_manifest.read_text())
        require(sdk['target'] == self.target, 'Wrong retained SDK target')
        for rel, expected in sdk['archives'].items():
            p = self.sdk / rel
            require(p.resolve().is_relative_to(self.sdk) and digest(p) == expected, f'SDK archive mismatch: {rel}')
            require(self.capture(['/usr/bin/lipo', '-archs', str(p)]) == 'arm64', f'SDK not single arm64: {rel}')
        for patch in sdk.get('sourcePatches', []) + sdk.get('dependencyPatches', []):
            require(digest(self.sdk / patch['path']) == patch['sha256'], 'SDK provenance patch mismatch')
        tooling = {name: self.capture(command) for name, command in {
            'rustc': ['rustc', '-Vv'], 'cargo': ['cargo', '-V'], 'node': ['node', '--version'],
            'xcode': ['/usr/bin/xcodebuild', '-version'], 'developer_dir': ['/usr/bin/xcode-select', '-p'],
            self.apple_sdk + '_sdk': ['/usr/bin/xcrun', '--sdk', self.apple_sdk, '--show-sdk-version']}.items()}
        tooling.update(self.wrapper_tooling())
        self.modules = self.repo / 'node_modules'
        require(self.modules.is_dir(), 'Existing node_modules is required; offline tool never installs packages')
        lock = json.loads(self.run(['/usr/bin/git', 'show', self.commit + ':package-lock.json']))
        installed_lock = self.modules / '.package-lock.json'
        require(installed_lock.is_file(), 'Missing installed npm lock identity')
        installed = json.loads(installed_lock.read_text())
        for name, item in lock['packages'].items():
            if not name or item.get('optional') and name not in installed['packages']:
                continue
            actual = installed['packages'].get(name)
            require(actual and actual.get('version') == item.get('version') and actual.get('integrity') == item.get('integrity'), f'npm cache differs from committed lock: {name}')
        signing = None
        self.signing = self.root / 'signing-inputs'
        # Keep one content-keyed signing snapshot, without changing the accepted app.
        if self.args.signing_source:
            source = Path(self.args.signing_source).expanduser().resolve(strict=True)
            require(source.suffix == '.app', 'Signing source must be an already accepted signed .app')
            self.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=2', str(source)])
            extensions = sorted((source / 'PlugIns').glob('*.appex'))
            require(len(extensions) == 1, 'Exactly one authorized widget extension is required')
            input_key = object_hash(tree(source))
            self.signing = self.signing / input_key
            self.signing.mkdir(parents=True, mode=0o700, exist_ok=True)
            main = self.profile(source, self.signing / 'main')
            widget = self.profile(extensions[0], self.signing / 'widget')
            require(main['team'] == widget['team'], 'Main/widget team mismatch')
            prefix = self.signing / 'accepted-cert-'
            self.run(['/usr/bin/codesign', '-d', '--extract-certificates=' + str(prefix), str(source)])
            signer = hashlib.sha1(Path(str(prefix) + '0').read_bytes()).hexdigest().upper()
            require(not self.args.signing_identity or self.args.signing_identity.upper() == signer, '--signing-identity must be the accepted app certificate SHA1')
            require(signer in main['certificate_sha1s'] and signer in widget['certificate_sha1s'], 'Signer certificate absent from profiles')
            identities = self.capture(['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning'])
            require(signer in identities, 'Accepted signing identity/private key not available locally')
            signing = {'main': main, 'widget': widget, 'signer_sha1': signer, 'widget_name': extensions[0].name}
        require(signing or self.args.prepare_only or self.args.unsigned or self.simulator, 'Pass --signing-source pointing to an accepted signed app')
        self.identity = {'schema': 1, 'platform': self.args.platform, 'rust_target': self.target, 'architectures': ['arm64'], 'source_commit': self.commit, 'sdk_manifest_sha256': digest(sdk_manifest),
                         'sdk_path': str(self.sdk), 'tooling': tooling, 'npm_installed_lock_sha256': digest(installed_lock),
                         'npm_committed_lock_sha256': hashlib.sha256(canonical(lock)).hexdigest(), 'signing': signing,
                         'helper_sha256': digest(Path(__file__).resolve())}
        self.fingerprint = object_hash(self.identity)
        self.work = self.root / 'builds' / self.fingerprint
        self.manifest = self.work / 'manifest.json'

    def prepare(self):
        if self.manifest.exists():
            manifest = json.loads(self.manifest.read_text())
            require(all(manifest.get(key) == value for key, value in self.identity.items()), 'Cached manifest identity changed')
            require(digest(self.work / 'source.tar') == manifest['source_archive_sha256'], 'Cached source archive changed')
            return manifest
        require(not self.work.exists(), f'Incomplete output exists; inspect it before choosing another --output: {self.work}')
        self.work.mkdir(parents=True, mode=0o700)
        self.source = self.work / 'source'
        self.source.mkdir()
        archive = self.work / 'source.tar'
        # git archive is the only source producer; never copy working tree files.
        self.run(['/usr/bin/git', 'archive', '--format=tar', '--output=' + str(archive), self.commit])
        with tarfile.open(archive) as tar:
            members = tar.getmembers()
            for member in members:
                path = PurePosixPath(member.name)
                require(not path.is_absolute() and '..' not in path.parts and (member.isfile() or member.isdir()), f'Unsafe archive entry: {member.name}')
            # All members were checked above; remain compatible with Python 3.9.
            tar.extractall(self.source, members=members)
        (self.source / 'node_modules').symlink_to(self.modules, target_is_directory=True)
        source_files = [[p.relative_to(self.source).as_posix(), digest(p)] for p in sorted(self.source.rglob('*')) if p.is_file() and not p.is_symlink() and 'node_modules' not in p.relative_to(self.source).parts]
        manifest = {**self.identity, 'status': 'prepared', 'fingerprint': self.fingerprint,
                    'source_archive_sha256': digest(archive), 'source_files': source_files,
                    'source_path': str(self.source), 'signing_inputs_path': str(self.signing), 'logs': self.commands}
        save(self.manifest, manifest)
        return manifest

    def build(self, manifest):
        self.source = Path(manifest['source_path'])
        # Keep Cargo's target tree under src-tauri so sherpa-onnx-sys can find
        # tauri.conf.json and copy its xcframework into the archived project.
        self.env['CARGO_TARGET_DIR'] = str(self.source / 'src-tauri/target')
        if manifest['status'] == 'ready':
            self.verify(manifest)
            return manifest
        require(manifest['status'] == 'prepared', 'Build is not resumable after failure; inspect owned output instead of retrying blindly')
        for rel, expected in manifest['source_files']:
            path = self.source / rel
            require(path.is_file() and not path.is_symlink() and digest(path) == expected, f'Prepared source changed: {rel}')
        manifest['status'] = 'building'
        save(self.manifest, manifest)
        config = self.work / 'vite-ios.config.mjs'
        # Merge the committed configuration (including ASCII output), not an older copied recipe.
        config.write_text('import { mergeConfig } from "vite";\nimport original from ' + json.dumps(str(self.source / 'vite.config.ts')) + ';\nexport default mergeConfig(original, ' + json.dumps({'root': str(self.source), 'cacheDir': str(self.work / 'vite-cache'), 'build': {'outDir': str(self.work / 'frontend-ios'), 'emptyOutDir': True}}) + ');\n')
        # Config must resolve Vite from the exact installed dependency tree.
        (self.work / 'node_modules').symlink_to(self.modules, target_is_directory=True)
        self.run(['node', str(self.modules / 'typescript/bin/tsc'), '--noEmit'], self.source)
        self.run(['node', str(self.modules / 'vite/bin/vite.js'), 'build', '--configLoader', 'runner', '--config', str(config)], self.source)
        frontend = tree(self.work / 'frontend-ios')
        require(frontend and (self.work / 'frontend-ios/index.html').is_file(), 'Missing production frontend')
        shutil.copytree(self.work / 'frontend-ios', self.source / 'dist')
        require(tree(self.source / 'dist') == frontend, 'Common dist differs from frozen iOS frontend')
        # sherpa-onnx's build script materializes its pinned iOS xcframework in
        # src-tauri before Xcode resolves the app target. Build that dependency
        # explicitly here; the later Tauri build reuses the same target cache.
        self.run(['cargo', 'build', '--package', 'sherpa-onnx-sys', '--target', self.target], self.source / 'src-tauri')
        metadata = json.loads(self.run(['cargo', 'metadata', '--offline', '--locked', '--format-version', '1', '--filter-platform', self.target], self.source / 'src-tauri'))
        packages = [p for p in metadata['packages'] if p['name'] == 'tauri']
        require(len(packages) == 1, 'Ambiguous resolved Tauri dependency')
        api = Path(packages[0]['manifest_path']).parent / 'mobile/ios-api'
        require((api / 'Package.swift').is_file(), 'Resolved Tauri package lacks iOS API sources')
        manifest['resolved_tauri'] = {'version': packages[0]['version'], 'manifest_path': packages[0]['manifest_path'], 'api_tree_sha256': object_hash(tree(api))}
        command = ['npm', 'run', 'tauri', '--', 'ios', 'build', '--target', 'aarch64-sim' if self.simulator else 'aarch64', '--ci', '--no-sign', '--archive-only', '--config', json.dumps({'build': {'beforeBuildCommand': '', 'frontendDist': str(self.work / 'frontend-ios')}})]
        self.run(command, self.source)
        generated = self.source / 'src-tauri/plugins/tauri-plugin-kiwix-mobile/.tauri/tauri-api'
        expected = [item for item in tree(api) if not any(item[0].startswith(prefix) for prefix in ('.build', 'Package.resolved', 'Tests'))]
        require(tree(generated) == expected, 'Generated plugin API differs from actual resolved Tauri sources')
        allowed = {'src-tauri/gen/apple/kiwix-workbench.xcodeproj/project.pbxproj', 'src-tauri/gen/apple/kiwix-workbench_iOS/Info.plist', 'src-tauri/gen/apple/kiwix-workbench_iOS/kiwix-workbench_iOS.entitlements', 'src-tauri/plugins/tauri-plugin-kiwix-mobile/ios/Package.resolved'}
        changes = []
        for rel, before in manifest['source_files']:
            after = digest(self.source / rel)
            if after != before:
                require(rel in allowed, f'Unexpected committed source mutation: {rel}')
                changes.append({'path': rel, 'before': before, 'after': after})
        require(tree(self.work / 'frontend-ios') == frontend and tree(self.source / 'dist') == frontend, 'Frontend mutated during native build')
        build_root = self.source / 'src-tauri/gen/apple/build'
        archives = list(build_root.rglob('*.xcarchive')) if self.simulator else list(build_root.glob('*.xcarchive'))
        if self.simulator:
            apps = list(build_root.glob('**/Release-iphonesimulator/*.app'))
            if not apps:
                apps = [app for archive in archives for app in (archive / 'Products/Applications').glob('*.app')]
        else:
            require(len(archives) == 1, 'Expected exactly one iOS archive')
            apps = list((archives[0] / 'Products/Applications').glob('*.app'))
        require(len(apps) == 1, 'Expected exactly one platform application')
        unsigned = self.work / 'unsigned' / 'Kiwix.app'
        unsigned.parent.mkdir()
        shutil.copytree(apps[0], unsigned, symlinks=True)
        manifest.update(status='unsigned', unsigned_app_path=str(unsigned), unsigned_artifact_sha256=object_hash(tree(unsigned)),
                        frontend_files=frontend, frontend_sha256=object_hash(frontend), generated_source_changes=changes,
                        build_command=command, build_environment=self.env, logs=self.commands, archive_path=str(archives[0]) if len(archives) == 1 else None)
        save(self.manifest, manifest)
        return manifest

    def sign(self, manifest):
        if self.simulator:
            return self.sign_simulator(manifest)
        if manifest['status'] == 'ready':
            self.verify(manifest)
            return manifest
        require(manifest['status'] == 'unsigned', 'Signing requires a complete unsigned build')
        unsigned = Path(manifest['unsigned_app_path'])
        require(object_hash(tree(unsigned)) == manifest['unsigned_artifact_sha256'], 'Unsigned artifact changed since build receipt')
        app = self.work / 'Kiwix.app'
        require(not app.exists(), 'Signing output exists; refuse automatic retry')
        shutil.copytree(unsigned, app, symlinks=True)
        signing = manifest['signing']
        require(signing, 'Missing frozen signing inputs')
        staged = Path(manifest['signing_inputs_path'])
        widget = app / 'PlugIns' / signing['widget_name']
        require(sorted(p.name for p in (app / 'PlugIns').iterdir()) == [signing['widget_name']], 'Unexpected nested extension')
        # Embedded frameworks (sherpa-onnx) must be signed before the enclosing
        # app; otherwise installd rejects the app with an invalid nested
        # signature even though the main bundle itself verifies.
        nested = sorted([*app.glob('Frameworks/**/*.framework'), *app.glob('Frameworks/**/*.dylib')],
                        key=lambda path: len(path.parts), reverse=True)
        for bundle in nested:
            self.run(['/usr/bin/codesign', '--force', '--sign', signing['signer_sha1'], '--timestamp=none', str(bundle)])
        # No --deep signing: nested code is signed first, then the widget, then
        # the enclosing app.
        for name, bundle in [('widget', widget), ('main', app)]:
            info = plistlib.loads((bundle / 'Info.plist').read_bytes())
            require(info['CFBundleIdentifier'] == signing[name]['bundle_id'], 'Built bundle ID differs from accepted signing source')
            profile = staged / name / 'embedded.mobileprovision'
            entitlements = staged / name / 'entitlements.plist'
            require(digest(profile) == signing[name]['profile_sha256'] and digest(entitlements) == signing[name]['entitlements_sha256'], 'Frozen signing input changed')
            shutil.copyfile(profile, bundle / 'embedded.mobileprovision')
            self.run(['/usr/bin/codesign', '--force', '--sign', signing['signer_sha1'], '--entitlements', str(entitlements), '--timestamp=none', str(bundle)])
        manifest.update(status='ready', app_path=str(app), bundle_id=signing['main']['bundle_id'], team=signing['main']['team'],
                        signer_sha1=signing['signer_sha1'], artifact_files=tree(app))
        manifest['artifact_sha256'] = object_hash(manifest['artifact_files'])
        manifest['bundles'] = self.verify(manifest)
        manifest['logs'] = self.commands
        save(self.manifest, manifest)
        return manifest

    def verify(self, manifest):
        require(manifest.get('platform') == self.args.platform, 'Artifact platform mismatch')
        if self.simulator:
            return self.verify_simulator(manifest)
        app = Path(manifest['app_path'])
        require(app.resolve().is_relative_to(self.work.resolve()), 'Artifact outside owned fingerprint output')
        require(object_hash(tree(app)) == manifest['artifact_sha256'], 'Signed app content changed')
        signing = manifest['signing']
        receipt = []
        bundles = [('widget', app / 'PlugIns' / signing['widget_name'])] if signing.get('widget_name') else []
        for name, bundle in [*bundles, ('main', app)]:
            self.run(['/usr/bin/codesign', '--verify', '--strict', '--verbose=2', str(bundle)])
            detail = self.capture(['/usr/bin/codesign', '-dv', '--verbose=4', str(bundle)], merged=True)
            require('TeamIdentifier=' + signing[name]['team'] in detail, 'Signed team mismatch')
            raw = self.run(['/usr/bin/codesign', '-d', '--entitlements', ':-', str(bundle)])
            begin, end = raw.find(b'<?xml'), raw.find(b'</plist>')
            require(begin >= 0 and end >= begin, 'Missing signed entitlements')
            require(plistlib.loads(raw[begin:end + 8]) == signing[name]['entitlements'], 'Signed entitlements changed')
            profile_raw = self.run(['/usr/bin/security', 'cms', '-D', '-i', str(bundle / 'embedded.mobileprovision')])
            profile = plistlib.loads(profile_raw)
            require(digest(bundle / 'embedded.mobileprovision') == signing[name]['profile_sha256'], 'Signed profile differs from accepted profile')
            require(profile['ExpirationDate'] > dt.datetime.now(dt.timezone.utc).replace(tzinfo=None), 'Signed profile expired')
            require(not self.args.device_udid or self.args.device_udid in profile.get('ProvisionedDevices', []), 'Device absent from signed profile')
            info = plistlib.loads((bundle / 'Info.plist').read_bytes())
            executable = bundle / info['CFBundleExecutable']
            require(executable.resolve().is_relative_to(bundle.resolve()), 'Escaping executable path')
            require(info['CFBundleIdentifier'] == signing[name]['bundle_id'], 'Signed bundle ID mismatch')
            archs = self.capture(['/usr/bin/lipo', '-archs', str(executable)]).split()
            require(archs == ['arm64'], 'Signed app must contain only arm64')
            prefix = self.work / ('verified-' + name + '-' + str(time.time_ns()) + '-')
            self.run(['/usr/bin/codesign', '-d', '--extract-certificates=' + str(prefix), str(bundle)])
            require(hashlib.sha1(Path(str(prefix) + '0').read_bytes()).hexdigest().upper() == signing['signer_sha1'], 'Actual signed certificate differs from accepted signer')
            receipt.append({'relative_path': bundle.relative_to(app).as_posix(), 'bundle_id': info['CFBundleIdentifier'],
                            'executable_sha256': digest(executable), 'architectures': archs, 'team': signing[name]['team'],
                            'profile_uuid': profile['UUID'], 'profile_expiration': signing[name]['profile_expiration'],
                            'provisioned_devices': profile.get('ProvisionedDevices', []), 'entitlements_sha256': signing[name]['entitlements_sha256'],
                            'strict_signature_verified': True})
        self.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=2', str(app)])
        for bundle in sorted([*app.glob('Frameworks/**/*.framework'), *app.glob('Frameworks/**/*.dylib')],
                             key=lambda path: len(path.parts), reverse=True):
            self.run(['/usr/bin/codesign', '--verify', '--strict', '--verbose=2', str(bundle)])
            prefix = self.work / ('verified-nested-' + str(time.time_ns()) + '-')
            self.run(['/usr/bin/codesign', '-d', '--extract-certificates=' + str(prefix), str(bundle)])
            require(hashlib.sha1(Path(str(prefix) + '0').read_bytes()).hexdigest().upper() == signing['signer_sha1'],
                    'Nested framework signature is not owned by the accepted signer')
        return receipt

    def simulator_binaries(self, app):
        """Inspect every shipped Mach-O, including frameworks and extensions."""
        binaries = []
        for path in sorted(app.rglob('*')):
            if not path.is_file() or path.is_symlink():
                continue
            with path.open('rb') as stream:
                magic = stream.read(4)
            if magic not in (b'\xcf\xfa\xed\xfe', b'\xfe\xed\xfa\xcf', b'\xca\xfe\xba\xbe', b'\xbe\xba\xfe\xca'):
                continue
            archs = self.capture(['/usr/bin/lipo', '-archs', str(path)]).split()
            require(archs == ['arm64'], f'Simulator binary must be single arm64: {path}')
            loads = self.capture(['/usr/bin/xcrun', 'vtool', '-show-build', str(path)])
            platforms = re.findall(r'^\s*platform\s+(\S+)', loads, re.MULTILINE)
            require(platforms and all(value in ('IOSSIMULATOR', '7') for value in platforms),
                    f'Binary is not an iOS simulator Mach-O: {path}')
            binaries.append({'relative_path': path.relative_to(app).as_posix(), 'sha256': digest(path),
                             'architectures': archs, 'platform': 'simulator', 'mach_o_build': loads})
        require(binaries, 'Simulator app has no Mach-O binaries')
        return binaries

    def sign_simulator(self, manifest):
        if manifest['status'] == 'ready':
            self.verify(manifest)
            return manifest
        require(manifest['status'] == 'unsigned', 'Simulator signing requires a real unsigned build')
        unsigned = Path(manifest['unsigned_app_path'])
        require(object_hash(tree(unsigned)) == manifest['unsigned_artifact_sha256'], 'Unsigned simulator app changed')
        self.simulator_binaries(unsigned)
        app = self.work / 'Kiwix.app'
        require(not app.exists(), 'Simulator signing output already exists')
        shutil.copytree(unsigned, app, symlinks=True)
        require(not list(app.rglob('embedded.mobileprovision')), 'Simulator app contains physical provisioning')
        nested = sorted([*app.glob('Frameworks/**/*.framework'), *app.glob('Frameworks/**/*.dylib'),
                         *app.glob('PlugIns/*.appex')], key=lambda path: len(path.parts), reverse=True)
        for bundle in [*nested, app]:
            self.run(['/usr/bin/codesign', '--force', '--sign', '-', '--timestamp=none', str(bundle)])
        info = plistlib.loads((app / 'Info.plist').read_bytes())
        manifest.update(status='ready', app_path=str(app), bundle_id=info['CFBundleIdentifier'],
                        signing=None, signing_kind='ad-hoc-simulator', provisioning_applicable=False,
                        artifact_files=tree(app))
        manifest['artifact_sha256'] = object_hash(manifest['artifact_files'])
        manifest['bundles'] = self.verify_simulator(manifest)
        manifest['logs'] = self.commands
        save(self.manifest, manifest)
        return manifest

    def verify_simulator(self, manifest):
        require(manifest.get('platform') == 'simulator', 'Missing simulator platform binding')
        app = Path(manifest['app_path'])
        require(app.resolve().is_relative_to(self.work.resolve()), 'Simulator artifact outside owned output')
        require(tree(app) == manifest['artifact_files'] and object_hash(tree(app)) == manifest['artifact_sha256'],
                'Simulator app inventory changed')
        require(not list(app.rglob('embedded.mobileprovision')), 'Simulator app contains physical provisioning')
        info = plistlib.loads((app / 'Info.plist').read_bytes())
        require(info['CFBundleIdentifier'] == manifest['bundle_id'] and
                info.get('DTPlatformName') == 'iphonesimulator' and
                info.get('CFBundleSupportedPlatforms') == ['iPhoneSimulator'], 'Simulator bundle metadata mismatch')
        self.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)])
        binaries = self.simulator_binaries(app)
        require(any(item['relative_path'] == info['CFBundleExecutable'] for item in binaries), 'Missing simulator app executable')
        if manifest.get('bundles'):
            require(manifest['bundles'] == binaries, 'Simulator Mach-O receipt changed')
        return binaries


class AppleBuild(Build):
    """Source-built upstream release, locally signed under a separate identity."""
    RELEASE = '3.18.0'
    COMMIT = '09a5ff1c9c3ffeda258ad593bbe16da612cfb149'
    UPSTREAM = 'https://github.com/kiwix/kiwix-apple.git'
    CORE_URL = 'https://download.kiwix.org/release/libkiwix/libkiwix_xcframework-14.2.1-2.tar.gz'
    CORE_MD5 = '78c0398d14228d11d2caf2c888314d04'
    # Resolve the release's declared ranges normally once, then retain the
    # actual lock and checkout commits. No synthetic minimum-version pins.

    def inputs(self):
        require(self.args.revision in ('HEAD', self.RELEASE, self.COMMIT),
                f'Upstream comparison currently pins released {self.RELEASE} ({self.COMMIT}); refusing a fork or unreviewed release')
        self.commit = self.COMMIT
        self.local_reference = None
        if self.args.upstream_repo:
            local = Path(self.args.upstream_repo).expanduser().resolve(strict=True)
            origin = self.capture(['/usr/bin/git', 'remote', 'get-url', 'origin'], local)
            require(origin.rstrip('/').removesuffix('.git') == self.UPSTREAM.removesuffix('.git'),
                    'Local reference origin is not official kiwix/kiwix-apple')
            self.local_reference = str(local)
        # A small, owned, single-release object store. Never fetch, checkout or
        # generate files in the user's fork, even when it is the local reference.
        self.git = self.root / 'upstream-release.git'
        if not self.git.exists():
            self.run(['/usr/bin/git', 'init', '--bare', str(self.git)])
            self.run(['/usr/bin/git', '--git-dir=' + str(self.git), 'fetch', '--depth=1', '--no-tags',
                      self.UPSTREAM, 'refs/tags/' + self.RELEASE + ':refs/tags/' + self.RELEASE])
        resolved = self.capture(['/usr/bin/git', '--git-dir=' + str(self.git), 'rev-parse',
                                 'refs/tags/' + self.RELEASE + '^{commit}'])
        require(resolved == self.COMMIT, 'Official release tag does not match reviewed immutable commit')
        brew = self.run(['/usr/bin/git', '--git-dir=' + str(self.git), 'show', self.commit + ':Brewfile']).decode()
        require(self.CORE_URL in brew, 'Release Brewfile no longer matches pinned CoreKiwix distribution')
        spec = self.run(['/usr/bin/git', '--git-dir=' + str(self.git), 'show', self.commit + ':project.yml']).decode()
        require('majorVersion: 8.2.0' in spec and 'majorVersion: 25.7.2' in spec and
                'revision: b083113aef646d9d35403ca17e9789b750e42d1d' in spec,
                'Release dependency requirements differ from reviewed upstream declarations')
        capabilities = set(re.findall(r'(com\.apple\.developer\.[A-Za-z0-9.-]+)\s*:', spec))
        require(capabilities == {'com.apple.developer.in-app-payments'},
                'Upstream declares capabilities outside the reviewed merchant-only signing limitation; obtain authorized profiles')
        require(self.simulator or self.args.allow_payment_entitlement_omission,
                'Official release requires merchant.org.kiwix.apple, which the existing wildcard profile cannot authorize. '
                'Preserve that capability with an authorized profile, or explicitly acknowledge reader-only comparison '
                'signing via --allow-payment-entitlement-omission; no source feature is patched.')
        self.core_archive = Path(self.args.corekiwix_archive).expanduser().resolve(strict=True) if self.args.corekiwix_archive else self.root / 'downloads' / self.CORE_URL.rsplit('/', 1)[1]
        if not self.core_archive.exists():
            self.core_archive.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            partial = self.core_archive.with_suffix(self.core_archive.suffix + '.download')
            require(not partial.exists() and not partial.is_symlink(), f'Interrupted download requires inspection: {partial}')
            print('iphone-build: Downloading official CoreKiwix SDK (300s maximum; fail if <100KiB/s for 30s)', file=sys.stderr, flush=True)
            self.run(['/usr/bin/curl', '--fail', '--location', '--proto', '=https', '--tlsv1.2',
                      '--connect-timeout', '15', '--speed-limit', '102400', '--speed-time', '30',
                      '--max-time', str(max(1, min(300, int(self.deadline - time.monotonic())))),
                      '--output', str(partial), self.CORE_URL])
            os.replace(partial, self.core_archive)
        checksum = hashlib.md5()
        with self.core_archive.open('rb') as stream:
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                checksum.update(chunk)
        require(checksum.hexdigest() == self.CORE_MD5, 'CoreKiwix archive differs from official published checksum')
        print('iphone-build: CoreKiwix SDK checksum verified; starting source/dependency preparation', file=sys.stderr, flush=True)
        self.core_sha256 = digest(self.core_archive)
        require(self.core_sha256 == '14820648788dddc3df8a89ff0820af60d7efc1cfda6c83602989455263ee1561',
                'CoreKiwix archive differs from reviewed official SHA256')
        sdk_manifest = self.root / 'downloads' / (self.core_sha256 + '.json')
        sdk_manifest.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        sdk_identity = {'schema': 1, 'source_url': self.CORE_URL, 'publisher_checksum_url': self.CORE_URL + '.md5',
                        'publisher_md5': self.CORE_MD5, 'archive_sha256': self.core_sha256,
                        'integrity_scope': 'Official HTTPS distribution and published MD5; locally recorded SHA256, not a publisher SHA256 claim'}
        if sdk_manifest.exists():
            require(json.loads(sdk_manifest.read_text()) == sdk_identity, 'CoreKiwix provenance receipt changed')
        else:
            save(sdk_manifest, sdk_identity)
        if self.simulator:
            bundle_id = self.args.upstream_bundle_id or 'com.tiltastech.zimreader.comparison.upstream'
            require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]+\.comparison\.[A-Za-z0-9.-]+', bundle_id),
                    'Simulator comparison needs a distinct .comparison. bundle identifier')
            entitlements, signing_main, signer = {}, None, None
            self.signing = self.root / 'simulator-signing'
        else:
            bundle_id, entitlements, signing_main, signer = self.physical_signing()
        python = self.args.upstream_python or (sys.executable if sys.version_info >= (3, 10) else
                  next((shutil.which(name) for name in ('python3.14', 'python3.13', 'python3.12', 'python3.11', 'python3.10') if shutil.which(name)), None))
        require(python, 'Official localization generator requires an existing Python >=3.10; pass --upstream-python')
        self.python = str(Path(python).expanduser().resolve(strict=True))
        tooling = {name: self.capture(command) for name, command in {
            'xcode': ['/usr/bin/xcodebuild', '-version'], 'developer_dir': ['/usr/bin/xcode-select', '-p'],
            self.apple_sdk + '_sdk': ['/usr/bin/xcrun', '--sdk', self.apple_sdk, '--show-sdk-version'],
            'swift': ['/usr/bin/xcrun', 'swift', '--version'], 'xcodegen': ['xcodegen', '--version'],
            'localization_python': [self.python, '--version']}.items()}
        tooling.update(self.wrapper_tooling())
        version = re.search(r'Python (\d+)\.(\d+)', tooling['localization_python'])
        require(version and tuple(map(int, version.groups())) >= (3, 10),
                'Official localization generator requires Python >=3.10; select an existing interpreter with --upstream-python')
        provenance = {'product': 'kiwix-apple', 'description': 'source-built upstream release; ad-hoc simulator signing' if self.simulator else 'source-built upstream release, locally developer-signed; not an App Store binary',
                      'upstream_url': self.UPSTREAM, 'release_tag': self.RELEASE, 'release_commit': self.commit,
                      'release_ref_authority': 'https://api.github.com/repos/kiwix/kiwix-apple/git/ref/tags/' + self.RELEASE,
                      'corekiwix_url': self.CORE_URL, 'corekiwix_publisher_checksum_url': self.CORE_URL + '.md5',
                      'corekiwix_publisher_md5': self.CORE_MD5, 'corekiwix_archive_sha256': self.core_sha256,
                      'dependency_policy': 'Resolve unchanged upstream-declared ranges normally on first build; retain actual lock and exact checkout commits thereafter. No App Store dependency parity claim.',
                      'local_reference_repo': self.local_reference,
                      'signing_only_overrides': {'bundle_id': bundle_id, 'display_name': 'Kiwix Upstream',
                                                 'entitlements': entitlements,
                                                 'payment_entitlement_omission_explicitly_acknowledged': self.args.allow_payment_entitlement_omission,
                                                 'limitation': 'Simulator: physical provisioning and merchant entitlement are not applicable; all upstream source retained.' if self.simulator else 'Original merchant entitlement cannot be authorized by the existing wildcard profile. Apple Pay donation capability is not claimed; reader source is unchanged.'}}
        self.identity = {'schema': 1, 'platform': self.args.platform, 'architectures': ['arm64'], 'rust_target': self.target, 'product': 'kiwix-apple', 'source_commit': self.commit,
                         'sdk_manifest_sha256': digest(sdk_manifest), 'sdk_manifest_path': str(sdk_manifest), 'tooling': tooling,
                         'signing': None if self.simulator else {'main': signing_main, 'signer_sha1': signer},
                         'provenance': provenance, 'helper_sha256': digest(Path(__file__).resolve())}
        if self.simulator:
            require(self.args.upstream_package_lock, 'Simulator builds require --upstream-package-lock with the existing frozen resolution receipt')
            lock_path = Path(self.args.upstream_package_lock).expanduser().resolve(strict=True)
            self.identity['frozen_package_receipt_sha256'] = digest(lock_path)
            self.identity['frozen_package_receipt_path'] = str(lock_path)
            self.identity['sourcepackages_path'] = str(Path(self.args.upstream_sourcepackages).expanduser().resolve(strict=True)) if self.args.upstream_sourcepackages else str(self.root / 'upstream-sourcepackages')
        self.fingerprint = object_hash(self.identity)
        self.work = self.root / 'upstream-builds' / self.fingerprint
        self.manifest = self.work / 'manifest.json'

    def physical_signing(self):
        require(self.args.signing_source, '--signing-source is required for the separately signed upstream comparison')
        accepted = Path(self.args.signing_source).expanduser().resolve(strict=True)
        self.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', '--verbose=2', str(accepted)])
        accepted_info = plistlib.loads((accepted / 'Info.plist').read_bytes())
        bundle_id = self.args.upstream_bundle_id or accepted_info['CFBundleIdentifier'] + '.comparison.upstream'
        require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]+', bundle_id) and '.' in bundle_id,
                'Invalid comparison bundle identifier')
        require(bundle_id != accepted_info['CFBundleIdentifier'] and
                bundle_id.startswith(accepted_info['CFBundleIdentifier'] + '.comparison.'),
                'Comparison ID must be inside the accepted app ID + .comparison. namespace; existing apps must not be replaced')
        candidates = sorted((accepted / 'PlugIns').glob('*.appex'))
        require(candidates, 'Accepted app must supply its already authorized wildcard-profile extension')
        self.signing = self.root / 'upstream-signing' / object_hash([object_hash(tree(accepted)), bundle_id])
        self.signing.mkdir(parents=True, exist_ok=True, mode=0o700)
        authorized = self.profile(candidates[0], self.signing / 'authorized')
        profile_path = self.signing / 'authorized/embedded.mobileprovision'
        profile = plistlib.loads(self.run(['/usr/bin/security', 'cms', '-D', '-i', str(profile_path)]))
        team = authorized['team']
        require(profile['Entitlements']['application-identifier'] == team + '.*',
                'Comparison requires the existing wildcard profile, not a new registration')
        entitlements = {'application-identifier': team + '.' + bundle_id,
                        'com.apple.developer.team-identifier': team, 'get-task-allow': True}
        require(profile['Entitlements'].get('get-task-allow') is True, 'Wildcard profile is not authorized for development signing')
        main = self.signing / 'main'
        main.mkdir(exist_ok=True, mode=0o700)
        for path in (main / 'embedded.mobileprovision', main / 'entitlements.plist'):
            require(not path.is_symlink(), f'Symlink signing output: {path}')
        shutil.copyfile(profile_path, main / 'embedded.mobileprovision')
        (main / 'entitlements.plist').write_bytes(plistlib.dumps(entitlements))
        prefix = self.signing / 'accepted-cert-'
        self.run(['/usr/bin/codesign', '-d', '--extract-certificates=' + str(prefix), str(accepted)])
        signer = hashlib.sha1(Path(str(prefix) + '0').read_bytes()).hexdigest().upper()
        require(not self.args.signing_identity or self.args.signing_identity.upper() == signer,
                'Explicit signer differs from accepted app certificate')
        require(signer in authorized['certificate_sha1s'], 'Accepted signing certificate absent from wildcard profile')
        require(signer in self.capture(['/usr/bin/security', 'find-identity', '-v', '-p', 'codesigning']),
                'Accepted signing private key unavailable locally')
        signing_main = {**authorized, 'bundle_id': bundle_id, 'entitlements': entitlements,
                        'entitlements_sha256': digest(main / 'entitlements.plist')}
        return bundle_id, entitlements, signing_main, signer

    def prepare(self):
        if self.manifest.exists():
            manifest = json.loads(self.manifest.read_text())
            require(manifest.get('input_identity') == self.identity, 'Upstream cached identity changed')
            require(digest(self.work / 'source.tar') == manifest['source_archive_sha256'], 'Upstream source archive changed')
            if manifest['status'] in ('unsigned', 'ready'):
                require(digest(Path(manifest['package_resolution_receipt_path'])) == manifest['package_resolution_receipt_sha256'],
                        'Frozen upstream dependency-resolution receipt changed')
            return manifest
        require(not self.work.exists(), f'Incomplete upstream staging requires inspection: {self.work}')
        self.work.mkdir(parents=True, mode=0o700)
        self.source = self.work / 'source'
        self.source.mkdir()
        archive = self.work / 'source.tar'
        self.run(['/usr/bin/git', '--git-dir=' + str(self.git), 'archive', '--format=tar',
                  '--output=' + str(archive), self.commit])
        with tarfile.open(archive) as tar:
            members = tar.getmembers()
            links = {PurePosixPath(member.name).as_posix() for member in members if member.issym()}
            for member in members:
                path = PurePosixPath(member.name)
                require(not path.is_absolute() and '..' not in path.parts and
                        (member.isfile() or member.isdir() or member.issym()),
                        f'Unsafe upstream source entry: {member.name}')
                require(not any(parent.as_posix() in links for parent in path.parents),
                        f'Archive entry traverses a symlink: {member.name}')
            # Extract regular members first, so no archive write can traverse
            # a link. Then retain only relative links to existing in-tree files.
            tar.extractall(self.source, members=[member for member in members if not member.issym()])
            for member in members:
                if not member.issym():
                    continue
                destination = self.source / member.name
                target = destination.parent / member.linkname
                require(not PurePosixPath(member.linkname).is_absolute() and
                        target.resolve().is_relative_to(self.source.resolve()) and
                        target.is_file() and not target.is_symlink(),
                        f'Unsafe upstream source link: {member.name} -> {member.linkname}')
                destination.symlink_to(member.linkname)
        original = tree(self.source)
        framework_root = self.root / 'corekiwix' / self.core_sha256
        framework = framework_root / 'CoreKiwix.xcframework'
        receipt = framework_root / 'receipt.json'
        if not receipt.exists():
            require(not framework_root.exists(), 'Incomplete framework extraction requires inspection')
            framework_root.mkdir(parents=True, mode=0o700)
            with tarfile.open(self.core_archive, 'r:gz') as tar:
                for member in tar.getmembers():
                    parts = PurePosixPath(member.name).parts
                    if 'CoreKiwix.xcframework' not in parts:
                        continue
                    relative = PurePosixPath(*parts[parts.index('CoreKiwix.xcframework'):])
                    require('..' not in relative.parts and not relative.is_absolute() and
                            (member.isdir() or member.isfile()), f'Unsafe framework member: {member.name}')
                    destination = framework_root / relative
                    if member.isdir():
                        destination.mkdir(parents=True, exist_ok=True)
                    else:
                        destination.parent.mkdir(parents=True, exist_ok=True)
                        with tar.extractfile(member) as source, destination.open('xb') as output:
                            shutil.copyfileobj(source, output)
                        os.chmod(destination, member.mode & 0o777)
            require((framework / 'Info.plist').is_file(), 'Official archive did not contain CoreKiwix.xcframework')
            # Apply only the exact module-map generation prescribed by Brewfile.
            modulemap = self.source / 'Support/CoreKiwix.modulemap'
            for slice_name in ('ios-arm64', 'ios-arm64_x86_64-simulator', 'macos-arm64_x86_64'):
                target = framework / slice_name / 'Headers/module.modulemap'
                require(target.parent.is_dir(), f'Missing official framework slice: {slice_name}')
                shutil.copyfile(modulemap, target)
            save(receipt, {'archive_sha256': self.core_sha256, 'files': tree(framework),
                           'modulemap_sha256': digest(modulemap)})
        frozen = json.loads(receipt.read_text())
        require(frozen['archive_sha256'] == self.core_sha256 and frozen['files'] == tree(framework) and
                frozen['modulemap_sha256'] == digest(self.source / 'Support/CoreKiwix.modulemap'),
                'Cached official framework content changed')
        metadata = plistlib.loads((framework / 'Info.plist').read_bytes())
        ios = [item for item in metadata['AvailableLibraries'] if item['SupportedPlatform'] == 'ios' and
               item.get('SupportedPlatformVariant') == ('simulator' if self.simulator else None)]
        require(len(ios) == 1 and 'arm64' in ios[0]['SupportedArchitectures'] and
                (self.simulator or ios[0]['SupportedArchitectures'] == ['arm64']), 'Official framework platform/architecture mismatch')
        library = framework / ios[0]['LibraryIdentifier'] / ios[0]['LibraryPath']
        require(library.resolve().is_relative_to(framework.resolve()), 'Escaping CoreKiwix library path')
        framework_archs = self.capture(['/usr/bin/lipo', '-archs', str(library)]).split()
        require('arm64' in framework_archs and (self.simulator or framework_archs == ['arm64']), 'CoreKiwix architecture mismatch')
        (self.source / 'CoreKiwix.xcframework').symlink_to(framework, target_is_directory=True)
        source_files = original
        manifest = {**self.identity, 'status': 'prepared', 'fingerprint': self.fingerprint,
                    'source_path': str(self.source), 'source_archive_sha256': digest(archive),
                    'source_files': source_files, 'original_source_files': original,
                    'signing_inputs_path': str(self.signing), 'framework_receipt_path': str(receipt),
                    'framework_binary_sha256': digest(library), 'logs': self.commands}
        manifest['framework_slice'] = {'library_identifier': ios[0]['LibraryIdentifier'],
                                     'platform': self.args.platform, 'architectures': framework_archs,
                                     'binary_sha256': digest(library), 'xcframework_info_sha256': digest(framework / 'Info.plist')}
        manifest['provenance'] = {**manifest['provenance'], 'source_configuration': {
            'project_yml_sha256': digest(self.source / 'project.yml'), 'change': 'None; original upstream dependency ranges retained'}}
        # identity excludes generated evidence but includes every producer input.
        manifest['input_identity'] = self.identity
        save(self.manifest, manifest)
        return manifest

    def validate_sources(self, manifest):
        for expected in manifest['source_files']:
            path = self.source / expected[0]
            require(path.resolve().is_relative_to(self.source.resolve()), f'Escaping upstream source: {expected[0]}')
            if expected[1] == 'link':
                require(path.is_symlink() and os.readlink(path) == expected[2] and path.is_file(),
                        f'Upstream source symlink changed: {expected[0]}')
            else:
                require(path.is_file() and not path.is_symlink() and digest(path) == expected[2] and
                        path.stat().st_mode & 0o111 == expected[3],
                        f'Upstream source file changed: {expected[0]}')

    def build(self, manifest):
        self.source = Path(manifest['source_path'])
        if manifest['status'] == 'ready':
            self.verify(manifest)
            return manifest
        require(manifest['status'] == 'prepared', 'Upstream build is not automatically retried after an unexpected failure')
        self.validate_sources(manifest)
        manifest['status'] = 'building'
        save(self.manifest, manifest)
        self.run([self.python, 'localizations.py', 'generate'], self.source)
        self.run(['xcodegen', 'generate', '--spec', 'project.yml'], self.source)
        resolved = self.source / 'Kiwix.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved'
        resolved.parent.mkdir(parents=True, exist_ok=True)
        packages = Path(self.args.upstream_sourcepackages).expanduser().resolve(strict=True) if self.args.upstream_sourcepackages else self.root / 'upstream-sourcepackages'
        derived = self.root / 'upstream-deriveddata'
        common = ['/usr/bin/xcodebuild', '-project', str(self.source / 'Kiwix.xcodeproj'), '-scheme', 'Kiwix',
                  '-clonedSourcePackagesDirPath', str(packages), '-derivedDataPath', str(derived)]
        resolution_receipt = Path(self.args.upstream_package_lock).expanduser().resolve(strict=True) if self.args.upstream_package_lock else self.root / 'upstream-package-locks' / (self.commit + '.json')
        frozen = json.loads(resolution_receipt.read_text()) if resolution_receipt.exists() else None
        require(not self.simulator or frozen, 'Simulator comparison requires an existing frozen upstream package receipt')
        require(not self.simulator or digest(resolution_receipt) == manifest['frozen_package_receipt_sha256'],
                'Frozen package resolution changed since simulator input snapshot')
        if frozen:
            require(frozen['source_commit'] == self.commit and frozen['project_yml_sha256'] == digest(self.source / 'project.yml'),
                    'Frozen dependency resolution belongs to different source requirements')
            save(resolved, frozen['lock'])
        resolve_command = [*common, '-resolvePackageDependencies']
        if frozen:
            resolve_command.append('-onlyUsePackageVersionsFromResolvedFile')
        self.run(resolve_command, self.source)
        lock = json.loads(resolved.read_text())
        actual = lock['pins']
        require(actual and all(pin.get('kind') == 'remoteSourceControl' and
                               re.fullmatch(r'[0-9a-f]{40}', pin.get('state', {}).get('revision', '')) for pin in actual),
                'Resolved dependencies lack immutable source-control revisions')
        if frozen:
            require(sorted(actual, key=lambda item: item['identity']) == sorted(frozen['lock']['pins'], key=lambda item: item['identity']),
                    'Resolution changed the previously frozen dependency commits')
        else:
            resolution_receipt.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            save(resolution_receipt, {'source_commit': self.commit, 'project_yml_sha256': digest(self.source / 'project.yml'),
                                      'resolved_at': dt.datetime.now(dt.timezone.utc).isoformat(), 'lock': lock})
        checkout_receipts = []
        for pin in actual:
            identity = pin['identity']
            checkout = packages / 'checkouts' / pin['location'].rsplit('/', 1)[1].removesuffix('.git')
            require(checkout.is_dir(), f'Missing pinned package checkout: {identity}')
            require(self.capture(['/usr/bin/git', 'rev-parse', 'HEAD'], checkout) == pin['state']['revision'],
                    f'Swift package checkout revision mismatch: {identity}')
            require(not self.capture(['/usr/bin/git', 'status', '--porcelain', '--untracked-files=all'], checkout),
                    f'Swift package checkout is dirty: {identity}')
            checkout_receipts.append({'identity': pin['identity'], 'revision': pin['state']['revision'],
                                      'location': pin['location']})
        signing = manifest['signing']
        bundle_id = manifest['provenance']['signing_only_overrides']['bundle_id']
        command = [*common, '-configuration', 'Release', '-sdk', self.apple_sdk, '-destination',
                   'generic/platform=iOS Simulator' if self.simulator else 'generic/platform=iOS',
                   '-jobs', os.environ.get('KIWIX_XCODE_JOBS', '4'), '-disableAutomaticPackageResolution', '-onlyUsePackageVersionsFromResolvedFile',
                   '-skipPackageUpdates', 'ARCHS=arm64', 'ONLY_ACTIVE_ARCH=YES', 'CODE_SIGNING_ALLOWED=NO',
                   'CODE_SIGNING_REQUIRED=NO', 'CODE_SIGN_IDENTITY=', 'DEVELOPMENT_TEAM=' + ('' if self.simulator else signing['main']['team']),
                   'PRODUCT_BUNDLE_IDENTIFIER=' + bundle_id,
                   'INFOPLIST_KEY_CFBundleDisplayName=Kiwix Upstream',
                   'CODE_SIGN_ENTITLEMENTS=' + ('' if self.simulator else str(Path(manifest['signing_inputs_path']) / 'main/entitlements.plist')),
                   'build']
        self.run(command, self.source)
        self.validate_sources(manifest)
        require(json.loads(resolved.read_text())['pins'] == actual, 'Build changed resolved package identities')
        built = derived / ('Build/Products/Release-' + self.apple_sdk) / 'Kiwix.app'
        require(built.is_dir() and not list((built / 'PlugIns').glob('*.appex')),
                'Unexpected upstream product/extension layout')
        info = plistlib.loads((built / 'Info.plist').read_bytes())
        require(info['CFBundleIdentifier'] == bundle_id and
                info['CFBundleShortVersionString'] == self.RELEASE, 'Built upstream identity/version mismatch')
        unsigned = self.work / 'unsigned/Kiwix.app'
        unsigned.parent.mkdir()
        shutil.copytree(built, unsigned, symlinks=True)
        manifest.update(status='unsigned', unsigned_app_path=str(unsigned), unsigned_artifact_sha256=object_hash(tree(unsigned)),
                        build_command=command, logs=self.commands, package_resolved_sha256=digest(resolved),
                        resolved_packages=checkout_receipts, derived_data_path=str(derived),
                        package_resolution_receipt_path=str(resolution_receipt),
                        package_resolution_receipt_sha256=digest(resolution_receipt))
        manifest['provenance'] = {**manifest['provenance'], 'spm_pins': actual,
                                  'package_resolved_sha256': digest(resolved),
                                  'package_resolution_receipt_sha256': digest(resolution_receipt)}
        save(self.manifest, manifest)
        return manifest

    def sign(self, manifest):
        if self.simulator:
            return self.sign_simulator(manifest)
        if manifest['status'] == 'ready':
            self.verify(manifest)
            return manifest
        require(manifest['status'] == 'unsigned', 'Upstream signing requires a complete unsigned receipt')
        unsigned = Path(manifest['unsigned_app_path'])
        require(object_hash(tree(unsigned)) == manifest['unsigned_artifact_sha256'], 'Unsigned upstream app changed')
        app = self.work / 'Kiwix.app'
        require(not app.exists(), 'Refusing to overwrite an existing upstream signing attempt')
        shutil.copytree(unsigned, app, symlinks=True)
        signing = manifest['signing']
        staged = Path(manifest['signing_inputs_path']) / 'main'
        require(digest(staged / 'embedded.mobileprovision') == signing['main']['profile_sha256'] and
                digest(staged / 'entitlements.plist') == signing['main']['entitlements_sha256'], 'Upstream signing inputs changed')
        shutil.copyfile(staged / 'embedded.mobileprovision', app / 'embedded.mobileprovision')
        nested = sorted([*app.glob('Frameworks/**/*.framework'), *app.glob('Frameworks/**/*.dylib')],
                        key=lambda path: len(path.parts), reverse=True)
        for binary in nested:
            require(binary.resolve().is_relative_to(app.resolve()), 'Escaping nested signing path')
            self.run(['/usr/bin/codesign', '--force', '--sign', signing['signer_sha1'], '--timestamp=none', str(binary)])
        self.run(['/usr/bin/codesign', '--force', '--sign', signing['signer_sha1'], '--timestamp=none',
                  '--entitlements', str(staged / 'entitlements.plist'), str(app)])
        manifest.update(status='ready', app_path=str(app), bundle_id=signing['main']['bundle_id'],
                        team=signing['main']['team'], signer_sha1=signing['signer_sha1'], artifact_files=tree(app))
        manifest['artifact_sha256'] = object_hash(manifest['artifact_files'])
        manifest['bundles'] = self.verify(manifest)
        manifest['logs'] = self.commands
        save(self.manifest, manifest)
        return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--repo', default=str(Path(__file__).resolve().parent.parent))
    parser.add_argument('--product', choices=('tauri', 'kiwix-apple'), default='tauri')
    parser.add_argument('--platform', choices=('device', 'simulator'), default='device',
                        help='Build physical iOS or arm64 iOS Simulator; simulator never uses physical credentials')
    parser.add_argument('--upstream-package-lock', help='Existing frozen upstream package-resolution JSON receipt')
    parser.add_argument('--upstream-sourcepackages', help='Existing shared Xcode SwiftPM cache; exact frozen checkouts required')
    parser.add_argument('--upstream-repo', help='Existing official-origin local reference repository; working tree remains untouched; source comes from a pinned owned upstream fetch')
    parser.add_argument('--upstream-bundle-id', help='Unique local comparison bundle ID; defaults to <accepted app ID>.comparison.upstream')
    parser.add_argument('--corekiwix-archive', help='Optional existing official 14.2.1-2 archive; publisher checksum is still mandatory')
    parser.add_argument('--upstream-python', help='Existing Python >=3.10 for the official localization generator')
    parser.add_argument('--allow-payment-entitlement-omission', action='store_true',
                        help='Explicitly acknowledge upstream Apple Pay merchant capability cannot be retained by the existing wildcard profile; reader source stays unchanged')
    parser.add_argument('--revision', default='HEAD', help='Commit-ish to archive; dirty working tree content is never included')
    parser.add_argument('--output', help='Owned persistent cache root; defaults to REPO/.native/iphone-tool')
    parser.add_argument('--sdk', help='Retained aarch64-apple-ios SDK root')
    parser.add_argument('--signing-source', help='Previously accepted signed .app; reuse exact profiles, entitlements and certificate')
    parser.add_argument('--signing-identity', help='Optional SHA1 of accepted signing certificate; must match the source app')
    parser.add_argument('--device-udid', help='Validate profile membership locally; never contacts this device')
    parser.add_argument('--timeout', type=int, default=3600, help='Total offline execution bound in seconds (default 3600)')
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument('--prepare-only', action='store_true', help='Freeze exact source and dependency/signing identities; do not build')
    actions.add_argument('--build', action='store_true', help='Build and sign the complete app (also the default action)')
    actions.add_argument('--sign', action='store_true', help='Sign existing unsigned app; do not build')
    actions.add_argument('--unsigned', action='store_true', help='Build unsigned app and stop for independent inspection')
    args = parser.parse_args()
    require(0 < args.timeout <= 7200, '--timeout must be between 1 and 7200 seconds')
    build = AppleBuild(args) if args.product == 'kiwix-apple' else Build(args)
    build.inputs()
    manifest = build.prepare()
    if args.prepare_only and manifest['status'] == 'ready':
        build.verify(manifest)
    if not args.prepare_only:
        if not args.sign:
            manifest = build.build(manifest)
        if not args.unsigned and (args.sign or manifest['status'] != 'ready'):
            manifest = build.sign(manifest)
    print(json.dumps({'manifest_path': str(build.manifest), 'status': manifest['status'], 'fingerprint': build.fingerprint}))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'iphone-build: {error}', file=sys.stderr)
        sys.exit(1)
