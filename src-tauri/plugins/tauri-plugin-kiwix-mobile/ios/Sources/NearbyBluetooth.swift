import Foundation
import CoreBluetooth
import UIKit
import Tauri
import os
import Darwin

struct NearbyBluetoothStartArgs: Decodable {
  let owner: String
  let role: String
  let publicId: String?
  let name: String?
}
struct NearbyBluetoothConnectArgs: Decodable { let owner: String; let peer: String }
struct NearbyBluetoothConnectionArgs: Decodable { let owner: String; let connection: String }
struct NearbyBluetoothSendArgs: Decodable { let owner: String; let connection: String; let bytes: String }
struct NearbyBluetoothScanArgs: Decodable { let owner: String; let enabled: Bool }
struct NearbyOwnerArgs: Decodable { let owner: String }

// All state, CoreBluetooth delegates and bridge delivery stay on the main serial queue.
// A permission sheet may make the scene inactive, but background/disconnect revokes it.
final class NearbyBluetooth {
  static let service = CBUUID(string: "7136d55f-2a4c-4df0-a75d-4d87f359d701")
  static let channel = CBUUID(string: "7136d55f-2a4c-4df0-a75d-4d87f359d702")
  static let info = CBUUID(string: "7136d55f-2a4c-4df0-a75d-4d87f359d703")
  private var central: NearbyBluetoothSession?
  private var peripheral: NearbyBluetoothSession?
  private let emit: (JSObject) -> Void

  private let currentScene: () -> UIScene?
  init(currentScene: @escaping () -> UIScene?, emit: @escaping (JSObject) -> Void) {
    self.currentScene = currentScene; self.emit = emit
  }

  func start(_ args: NearbyBluetoothStartArgs, scene: UIScene?, invoke: Invoke) {
    guard UUID(uuidString: args.owner)?.uuidString.lowercased() == args.owner,
          args.role == "central" || args.role == "peripheral" else {
      invoke.reject("BLE_INVALID_REQUEST"); return
    }
    guard let scene = scene, scene.activationState == .foregroundActive,
          UIApplication.shared.applicationState != .background else {
      invoke.reject("BLE_FOREGROUND_REQUIRED"); return
    }
    guard (args.role == "central" ? central : peripheral) == nil, session(args.owner) == nil else {
      invoke.reject("BLE_OWNER_BUSY"); return
    }
    var info: Data?
    if args.role == "peripheral" {
      guard let raw = args.publicId, let id = UUID(uuidString: raw),
            id.uuidString.lowercased() == raw, let name = args.name,
            !name.isEmpty, name.utf8.count <= 256 else {
        invoke.reject("BLE_INVALID_REQUEST"); return
      }
      var uuid = id.uuid
      info = Data([2, 0]) + withUnsafeBytes(of: &uuid) { Data($0) }
    } else if args.publicId != nil { invoke.reject("BLE_INVALID_REQUEST"); return }
    let session = NearbyBluetoothSession(owner: args.owner, scene: scene, info: info,
      name: args.name, currentScene: currentScene,
      emit: emit, ended: { [weak self] old in
        if self?.central === old { self?.central = nil }
        if self?.peripheral === old { self?.peripheral = nil }
      })
    if args.role == "central" { central = session } else { peripheral = session }
    session.start(invoke)
  }

  private func session(_ owner: String) -> NearbyBluetoothSession? {
    if central?.owner == owner { return central }
    if peripheral?.owner == owner { return peripheral }
    return nil
  }
  func connect(_ args: NearbyBluetoothConnectArgs, invoke: Invoke) {
    guard central?.owner == args.owner else { invoke.reject("BLE_OWNER_RETIRED"); return }
    central?.connect(args.peer, invoke: invoke)
  }
  func send(_ args: NearbyBluetoothSendArgs, invoke: Invoke) {
    guard let session = session(args.owner), args.bytes.utf8.count <= 684,
          let bytes = Data(base64Encoded: args.bytes), bytes.base64EncodedString() == args.bytes else {
      invoke.reject("BLE_INVALID_REQUEST"); return
    }
    session.send(args.connection, bytes: bytes, invoke: invoke)
  }
  func disconnect(_ args: NearbyBluetoothConnectionArgs, invoke: Invoke) {
    session(args.owner)?.disconnect(args.connection)
    invoke.resolve()
  }
  func scan(_ owner: String, enabled: Bool, invoke: Invoke) {
    guard let session = central, session.owner == owner else { invoke.reject("BLE_OWNER_RETIRED"); return }
    session.scan(enabled); invoke.resolve()
  }
  func stop(_ owner: String, invoke: Invoke) {
    session(owner)?.retire("BLE_CANCELLED", report: false)
    invoke.resolve()
  }
  func retire(scene: UIScene? = nil) {
    if scene == nil || central?.owns(scene) == true { central?.retire("BLE_BACKGROUND", report: true) }
    if scene == nil || peripheral?.owns(scene) == true { peripheral?.retire("BLE_BACKGROUND", report: true) }
  }
  static func encodeURL(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
  }
}

/// Bounded diagnostic file for the Nearby BLE and hotspot paths.
///
/// The file handle is opened once and reused, appends are serialized on a
/// private queue, and writes are buffered until the flush threshold or cadence
/// is reached. Per-packet CoreBluetooth callbacks therefore never pay a file
/// open/stat/seek/close cycle or a stdout flush. The acceptance app is a
/// release build with no environment override, so this file is always kept.
final class NearbyDiagnosticLog {
  static let shared = NearbyDiagnosticLog()
  private static let limit = 512 * 1024
  private static let flushThreshold = 32 * 1024
  private static let flushInterval: TimeInterval = 2
  private static let fileURL: URL? = {
    guard let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first,
          let identifier = Bundle.main.bundleIdentifier else { return nil }
    let directory = base.appendingPathComponent(identifier, isDirectory: true)
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory.appendingPathComponent("nearby-debug.log")
  }()
  private let queue = DispatchQueue(label: "org.kiwix.mobile.nearby.diagnostics", qos: .utility)
  private var handle: FileHandle?
  private var buffer = Data()
  private var written = 0
  private var openFailed = false
  private var timer: DispatchSourceTimer?

  private init() {
    for name in [UIApplication.didEnterBackgroundNotification, UIApplication.willTerminateNotification] {
      NotificationCenter.default.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
        self?.flush()
      }
    }
  }

  func append(_ line: String) {
    queue.async { self.appendLocked(line) }
  }

  /// Writes everything buffered so far; callers flush at termination/background.
  func flush() {
    queue.sync { self.flushLocked() }
  }

  private func appendLocked(_ line: String) {
    buffer.append(contentsOf: line.utf8)
    if buffer.count > Self.limit {
      // Drop half the cap at once so a full buffer does not memmove the whole
      // 512 KiB on every subsequent line.
      buffer.removeFirst(buffer.count - Self.limit / 2)
    }
    if !openFailed && buffer.count >= Self.flushThreshold { flushLocked() }
    startTimer()
  }

  private func startTimer() {
    guard timer == nil else { return }
    let timer = DispatchSource.makeTimerSource(queue: queue)
    timer.schedule(deadline: .now() + Self.flushInterval, repeating: Self.flushInterval, leeway: .seconds(1))
    timer.setEventHandler { [weak self] in self?.flushLocked() }
    timer.resume()
    self.timer = timer
  }

  private func stopTimer() {
    timer?.cancel()
    timer = nil
  }

  private func flushLocked() {
    guard !buffer.isEmpty else { stopTimer(); return }
    guard openHandle() else {
      openFailed = true
      return
    }
    openFailed = false
    // The Rust nearby writer may append between flushes, so read the real
    // size instead of trusting a private counter.
    written = Int((try? handle?.seekToEnd()) ?? 0)
    let data = buffer
    if written + data.count > Self.limit {
      // Rotate instead of growing without bound. Rename, never unlink: the
      // Rust writer may still hold this inode open.
      try? handle?.close(); handle = nil
      if let url = Self.fileURL {
        let previous = url.deletingLastPathComponent().appendingPathComponent("nearby-debug.previous.log")
        try? FileManager.default.removeItem(at: previous)
        try? FileManager.default.moveItem(at: url, to: previous)
      }
      written = 0
      guard openHandle() else { return }
    }
    buffer.removeAll(keepingCapacity: true)
    do {
      try handle?.write(contentsOf: data)
      written += data.count
      if buffer.isEmpty { stopTimer() }
    } catch {
      // Retry with a fresh descriptor on the next flush.
      try? handle?.close(); handle = nil
    }
  }

  private func openHandle() -> Bool {
    if handle != nil { return true }
    guard let url = Self.fileURL else { return false }
    // O_APPEND is required: the Rust nearby_log writer appends to this same
    // path. A plain FileHandle offset would overwrite those bytes.
    let fd = open(url.path, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0o600)
    guard fd >= 0 else { return false }
    let opened = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    written = Int((try? opened.seekToEnd()) ?? 0)
    handle = opened
    return true
  }
}

private final class NearbyBluetoothSession: NSObject, CBCentralManagerDelegate, CBPeripheralManagerDelegate {
  let owner: String
  private let idleTimer = MobileIdleTimerLease()
  private weak var scene: UIScene?
  private let publicInfo: Data?
  private let name: String?
  private let log = Logger(subsystem: "org.kiwix.workbench", category: "Nearby")
  private func debugLog(_ message: String) {
    log.info("\(message, privacy: .public)")
    #if DEBUG
    print("[nearby] \(message)")
    fflush(stdout)
    #endif
    NearbyDiagnosticLog.shared.append("[nearby] \(message)\n")
  }
  private var scanning = true
  private let emit: (JSObject) -> Void
  private let ended: (NearbyBluetoothSession) -> Void
  private var central: CBCentralManager?
  private var peripheral: CBPeripheralManager?
  private var retired = false
  private var sequence: UInt64 = 0
  private var startInvoke: Invoke?
  private var connectInvoke: Invoke?
  private var sendInvoke: Invoke?
  private var promptTimer: DispatchWorkItem?
  private var connectTimer: DispatchWorkItem?
  private var sendTimer: DispatchWorkItem?
  private var peers: [String: CBPeripheral] = [:]
  private var seenSources: Set<UUID> = []
  private var unnamedSources: [UUID: String] = [:]
  private var selected: CBPeripheral?
  private var selectedPeerHandle: String?
  private var clientDelegate: NearbyBluetoothConnectionDelegate?
  private let currentScene: () -> UIScene?
  private var subscriber: CBCentral?
  private var connection: String?
  private var channel: CBCharacteristic?
  private var information: CBCharacteristic?
  private var serverChannel: CBMutableCharacteristic?
  private var serverInformation: CBMutableCharacteristic?
  private var serverService: CBMutableService?
  private var receivedInfo: Data?
  private var budget = 0
  private var ready = false
  private var pendingIndication: Data?

  init(owner: String, scene: UIScene, info: Data?, name: String?, currentScene: @escaping () -> UIScene?,
       emit: @escaping (JSObject) -> Void, ended: @escaping (NearbyBluetoothSession) -> Void) {
    self.owner = owner; self.scene = scene; self.publicInfo = info
    self.name = name
    self.currentScene = currentScene; self.emit = emit; self.ended = ended
  }
  func owns(_ candidate: UIScene?) -> Bool { scene === candidate }
  func ownsConnection(_ candidate: CBPeripheral, handle: String) -> Bool {
    live && selected === candidate && connection == handle
  }
  private var live: Bool {
    !retired && scene != nil && currentScene() === scene && scene?.activationState != .background
      && scene?.activationState != .unattached && UIApplication.shared.applicationState != .background
  }
  func start(_ invoke: Invoke) {
    debugLog("start role=\(publicInfo == nil ? "central" : "peripheral")")
    startInvoke = invoke
    let timer = DispatchWorkItem { [weak self] in self?.retire("BLE_PERMISSION_TIMEOUT", report: true) }
    promptTimer = timer; DispatchQueue.main.asyncAfter(deadline: .now() + 60, execute: timer)
    if publicInfo == nil {
      central = CBCentralManager(delegate: self, queue: .main, options: [CBCentralManagerOptionShowPowerAlertKey: true])
    } else {
      peripheral = CBPeripheralManager(delegate: self, queue: .main, options: [CBPeripheralManagerOptionShowPowerAlertKey: true])
    }
  }
  private func event(_ kind: String, _ fields: JSObject = [:]) {
    guard !retired, sequence < UInt64.max else { return }
    sequence += 1
    var value = fields; value["owner"] = owner; value["sequence"] = NSNumber(value: sequence); value["kind"] = kind
    emit(value)
  }
  private func started() {
    promptTimer?.cancel(); promptTimer = nil
    let invoke = startInvoke; startInvoke = nil
    invoke?.resolve(["owner": owner, "started": true])
  }
  private func state(_ state: CBManagerState) {
    debugLog("manager state=\(state.rawValue)")
    guard live else { retire("BLE_BACKGROUND", report: true); return }
    switch state {
    case .poweredOn: break
    case .unknown: return
    case .resetting: retire("BLE_RESET", report: true)
    case .unauthorized: retire("BLE_PERMISSION_DENIED", report: true)
    case .unsupported: retire("BLE_UNSUPPORTED", report: true)
    case .poweredOff: retire("BLE_POWERED_OFF", report: true)
    @unknown default: retire("BLE_UNAVAILABLE", report: true)
    }
  }
  func centralManagerDidUpdateState(_ manager: CBCentralManager) {
    guard central === manager, !retired else { return }
    state(manager.state)
    guard live, manager.state == .poweredOn else { return }
    if scanning { manager.scanForPeripherals(withServices: [NearbyBluetooth.service], options: [CBCentralManagerScanOptionAllowDuplicatesKey: true]) }
    started()
  }
  func peripheralManagerDidUpdateState(_ manager: CBPeripheralManager) {
    guard peripheral === manager, !retired else { return }
    state(manager.state)
    guard live, manager.state == .poweredOn, serverService == nil, publicInfo != nil else { return }
    let channel = CBMutableCharacteristic(type: NearbyBluetooth.channel, properties: [.write, .indicate], value: nil, permissions: [.writeable])
    let information = CBMutableCharacteristic(type: NearbyBluetooth.info, properties: [.read], value: nil, permissions: [.readable])
    let service = CBMutableService(type: NearbyBluetooth.service, primary: true)
    service.characteristics = [channel, information]
    serverChannel = channel; serverInformation = information; serverService = service
    manager.add(service)
  }
  func peripheralManager(_ manager: CBPeripheralManager, didAdd service: CBService, error: Error?) {
    guard peripheral === manager, serverService === service, live else { return }
    guard error == nil else { retire("BLE_SERVICE_FAILED", report: true); return }
    var advertisement: [String: Any] = [CBAdvertisementDataServiceUUIDsKey: [NearbyBluetooth.service]]
    if let name = name {
      var prefix = ""
      for character in name {
        let next = prefix + String(character)
        if next.utf8.count > 8 { break }
        prefix = next
      }
      if !prefix.isEmpty { advertisement[CBAdvertisementDataLocalNameKey] = prefix }
    }
    manager.startAdvertising(advertisement)
  }
  func peripheralManagerDidStartAdvertising(_ manager: CBPeripheralManager, error: Error?) {
    guard peripheral === manager, live else { return }
    if error != nil { retire("BLE_ADVERTISE_FAILED", report: true) } else { started() }
  }
  func centralManager(_ manager: CBCentralManager, didDiscover candidate: CBPeripheral, advertisementData: [String: Any], rssi RSSI: NSNumber) {
    guard central === manager, live, scanning, selected == nil else { return }
    let existing = unnamedSources[candidate.identifier]
    guard existing != nil || (!seenSources.contains(candidate.identifier) && seenSources.count < 16) else { return }
    var advertisedName: String?
    if let name = advertisementData[CBAdvertisementDataLocalNameKey] as? String,
       !name.isEmpty, name.utf8.count <= 256 { advertisedName = name }
    if advertisedName == nil, let data = advertisementData[CBAdvertisementDataServiceDataKey] as? [CBUUID: Data],
       let bytes = data[NearbyBluetooth.service], !bytes.isEmpty, bytes.count <= 8 {
      advertisedName = String(data: bytes, encoding: .utf8)
    }
    // Android's name hint arrives in the scan response, which may follow the
    // first service advertisement. Upgrade that same peer once, never add a row.
    if existing != nil && advertisedName == nil { return }
    let handle = existing ?? UUID().uuidString.lowercased()
    if existing == nil {
      seenSources.insert(candidate.identifier)
      peers[handle] = candidate
      if advertisedName == nil { unnamedSources[candidate.identifier] = handle }
    } else {
      unnamedSources.removeValue(forKey: candidate.identifier)
    }
    var fields: JSObject = ["peer": handle]
    if let name = advertisedName { fields["name"] = name }
    let rawName = fields["name"] as? String ?? "-"
    let safeName = String(String.UnicodeScalarView(rawName.unicodeScalars.filter {
      !CharacterSet.controlCharacters.contains($0) && !CharacterSet.newlines.contains($0)
    }.prefix(64)))
    debugLog("peer name=\(safeName)")
    event("peer", fields)
  }
  func connect(_ peer: String, invoke: Invoke) {
    guard live, startInvoke == nil, let manager = central, manager.state == .poweredOn,
          let known = peers[peer],
          let candidate = manager.retrievePeripherals(withIdentifiers: [known.identifier]).first else {
      invoke.reject("BLE_PEER_UNAVAILABLE"); return
    }
    guard selected == nil, connectInvoke == nil else { invoke.reject("BLE_CONNECTION_BUSY"); return }
    debugLog("connect peer=\(peer)")
    manager.stopScan(); selected = candidate; selectedPeerHandle = peer
    let handle = UUID().uuidString.lowercased()
    connection = handle; connectInvoke = invoke
    let delegate = NearbyBluetoothConnectionDelegate(session: self, handle: handle)
    clientDelegate = delegate; candidate.delegate = delegate
    let timer = DispatchWorkItem { [weak self] in self?.failConnection("BLE_CONNECT_TIMEOUT") }
    connectTimer = timer; DispatchQueue.main.asyncAfter(deadline: .now() + 30, execute: timer)
    manager.connect(candidate, options: nil)
  }
  func centralManager(_ manager: CBCentralManager, didConnect candidate: CBPeripheral) {
    guard central === manager, selected === candidate, live else { return }
    debugLog("central connected")
    candidate.discoverServices([NearbyBluetooth.service])
  }
  func centralManager(_ manager: CBCentralManager, didFailToConnect candidate: CBPeripheral, error: Error?) {
    guard central === manager, selected === candidate, !retired else { return }
    failConnection("BLE_CONNECT_FAILED")
  }
  func centralManager(_ manager: CBCentralManager, didDisconnectPeripheral candidate: CBPeripheral, error: Error?) {
    guard central === manager, selected === candidate, !retired else { return }
    failConnection("BLE_DISCONNECTED")
  }
  func peripheral(_ candidate: CBPeripheral, didDiscoverServices error: Error?) {
    guard selected === candidate, live else { return }
    debugLog("services=\(candidate.services?.count ?? 0)")
    guard error == nil, let services = candidate.services, services.count == 1,
          let service = services.first, service.uuid == NearbyBluetooth.service else { failConnection("BLE_SERVICE_INVALID"); return }
    candidate.discoverCharacteristics([NearbyBluetooth.channel, NearbyBluetooth.info], for: service)
  }
  func peripheral(_ candidate: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
    guard selected === candidate, live, service.peripheral === candidate, service.uuid == NearbyBluetooth.service else { return }
    guard error == nil, let values = service.characteristics, values.count == 2,
          let channel = values.first(where: { $0.uuid == NearbyBluetooth.channel }),
          let info = values.first(where: { $0.uuid == NearbyBluetooth.info }),
          channel.properties == [.write, .indicate],
          info.properties == [.read] else { failConnection("BLE_CHARACTERISTICS_INVALID"); return }
    debugLog("characteristics=\(values.count)")
    self.channel = channel; information = info
    candidate.readValue(for: info)
  }
  func peripheral(_ candidate: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
    debugLog("value uuid=\(characteristic.uuid.uuidString) bytes=\(characteristic.value?.count ?? -1) error=\(error?.localizedDescription ?? "-")")
    guard selected === candidate, live else { return }
    guard error == nil, let bytes = characteristic.value else { failConnection("BLE_READ_FAILED"); return }
    if information === characteristic {
      guard receivedInfo == nil, bytes.count == 18, bytes[0] == 2, bytes[1] == 0,
            let channel = channel else { failConnection("BLE_INFO_INVALID"); return }
      receivedInfo = bytes; candidate.setNotifyValue(true, for: channel)
    } else if channel === characteristic {
      guard ready, bytes.count >= 9, bytes.count <= budget, let connection = connection else { failConnection("BLE_PACKET_INVALID"); return }
      event("bytes", ["connection": connection, "bytes": bytes.base64EncodedString()])
    } else { failConnection("BLE_SOURCE_INVALID") }
  }
  func peripheral(_ candidate: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
    debugLog("notify uuid=\(characteristic.uuid.uuidString) notifying=\(characteristic.isNotifying) error=\(error?.localizedDescription ?? "-")")
    guard selected === candidate, channel === characteristic, live else { return }
    guard !ready, error == nil, characteristic.isNotifying, let info = receivedInfo,
          let connection = connection else { failConnection("BLE_SUBSCRIBE_FAILED"); return }
    // With-response may include prepared writes. Query the single-ATT budget
    // without ever using a write-without-response operation on the wire.
    budget = min(512, candidate.maximumWriteValueLength(for: .withResponse),
      candidate.maximumWriteValueLength(for: .withoutResponse))
    guard budget >= 20 else { failConnection("BLE_PACKET_BUDGET"); return }
    ready = true; connectTimer?.cancel(); connectTimer = nil
    selectedPeerHandle = nil
    debugLog("ready budget=\(budget)")
    let reply: JSObject = ["owner": owner, "connection": connection, "info": NearbyBluetooth.encodeURL(info), "packetBudget": budget]
    let invoke = connectInvoke; connectInvoke = nil; invoke?.resolve(reply)
  }
  func send(_ handle: String, bytes: Data, invoke: Invoke) {
    guard live, ready, connection == handle else { invoke.reject("BLE_CONNECTION_RETIRED"); return }
    guard bytes.count >= 9, bytes.count <= budget else { invoke.reject("BLE_PACKET_INVALID"); return }
    guard sendInvoke == nil else { invoke.reject("BLE_BACKPRESSURE"); return }
    sendInvoke = invoke
    let timer = DispatchWorkItem { [weak self] in self?.failConnection("BLE_SEND_TIMEOUT") }
    sendTimer = timer; DispatchQueue.main.asyncAfter(deadline: .now() + 10, execute: timer)
    if let candidate = selected, let channel = channel {
      candidate.writeValue(bytes, for: channel, type: .withResponse)
    } else if let manager = peripheral, let channel = serverChannel, let subscriber = subscriber {
      if manager.updateValue(bytes, for: channel, onSubscribedCentrals: [subscriber]) {
        accepted()
      } else { pendingIndication = bytes }
    } else { failConnection("BLE_CONNECTION_RETIRED") }
  }
  private func accepted() {
    sendTimer?.cancel(); sendTimer = nil; pendingIndication = nil
    let invoke = sendInvoke; sendInvoke = nil; invoke?.resolve(["accepted": true])
  }
  func peripheral(_ candidate: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
    guard selected === candidate, channel === characteristic, live, sendInvoke != nil else { return }
    if error != nil { failConnection("BLE_WRITE_FAILED") } else { accepted() }
  }
  func peripheralManagerIsReady(toUpdateSubscribers manager: CBPeripheralManager) {
    guard peripheral === manager, live, let bytes = pendingIndication,
          let channel = serverChannel, let subscriber = subscriber else { return }
    if manager.updateValue(bytes, for: channel, onSubscribedCentrals: [subscriber]) { accepted() }
  }
  func peripheralManager(_ manager: CBPeripheralManager, central candidate: CBCentral, didSubscribeTo characteristic: CBCharacteristic) {
    guard peripheral === manager, serverChannel === characteristic, live else { return }
    guard subscriber == nil, let info = publicInfo else { return }
    let valueBudget = min(512, candidate.maximumUpdateValueLength)
    guard valueBudget >= 20 else { return }
    subscriber = candidate; budget = valueBudget; ready = true
    let handle = UUID().uuidString.lowercased(); connection = handle
    event("connected", ["connection": handle, "info": NearbyBluetooth.encodeURL(info), "packetBudget": budget])
  }
  func peripheralManager(_ manager: CBPeripheralManager, central candidate: CBCentral, didUnsubscribeFrom characteristic: CBCharacteristic) {
    guard peripheral === manager, serverChannel === characteristic, subscriber === candidate, !retired else { return }
    resetPeripheral("BLE_DISCONNECTED")
  }
  func peripheralManager(_ manager: CBPeripheralManager, didReceiveRead request: CBATTRequest) {
    guard peripheral === manager, live, serverInformation === request.characteristic,
          request.offset == 0, let info = publicInfo else {
      manager.respond(to: request, withResult: .requestNotSupported); return
    }
    request.value = info; manager.respond(to: request, withResult: .success)
  }
  func peripheralManager(_ manager: CBPeripheralManager, didReceiveWrite requests: [CBATTRequest]) {
    guard let request = requests.first else { return }
    guard peripheral === manager, live, ready, requests.count == 1,
          serverChannel === request.characteristic, subscriber === request.central,
          request.offset == 0, let bytes = request.value, bytes.count >= 9, bytes.count <= budget,
          let connection = connection else {
      manager.respond(to: request, withResult: .requestNotSupported); return
    }
    event("bytes", ["connection": connection, "bytes": bytes.base64EncodedString()])
    manager.respond(to: request, withResult: .success)
  }
  func disconnect(_ handle: String) {
    guard connection == handle else { return }
    failConnection("BLE_DISCONNECTED")
  }
  private func failConnection(_ code: String) {
    guard !retired else { return }
    debugLog("fail \(code) peer=\(selectedPeerHandle ?? "-")")
    guard let manager = central else { resetPeripheral(code); return }
    // A fresh manager and per-connection delegate fence callbacks independently
    // of the public candidate handle retained after a completed BLE exchange.
    guard let selected = selected else { return }
    let oldConnection = connection
    if let peer = selectedPeerHandle {
        // The failed candidate may be a stale advertisement. Drop it so the
        // next attempt cannot keep timing out against the same peer.
        peers.removeValue(forKey: peer)
        unnamedSources.removeValue(forKey: selected.identifier)
        event("lost", ["peer": peer])
    }
    selectedPeerHandle = nil
    self.selected = nil; connection = nil; ready = false; budget = 0
    selected.delegate = nil; clientDelegate = nil
    manager.delegate = nil; manager.stopScan(); manager.cancelPeripheralConnection(selected)
    central = nil
    channel = nil; information = nil; receivedInfo = nil; pendingIndication = nil
    connectTimer?.cancel(); sendTimer?.cancel(); connectTimer = nil; sendTimer = nil
    let pending = [connectInvoke, sendInvoke]; connectInvoke = nil; sendInvoke = nil
    pending.forEach { $0?.reject(code) }
    if let handle = oldConnection { event("disconnected", ["connection": handle, "error": code]) }
    if live {
      central = CBCentralManager(delegate: self, queue: .main, options: [CBCentralManagerOptionShowPowerAlertKey: false])
    } else { retire("BLE_BACKGROUND", report: true) }
  }
  func scan(_ enabled: Bool) {
    scanning = enabled
    guard live, let manager = central, manager.state == .poweredOn else { return }
    if enabled && selected == nil {
      manager.scanForPeripherals(withServices: [NearbyBluetooth.service], options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
    } else { manager.stopScan() }
  }
  private func resetPeripheral(_ code: String) {
    guard !retired, peripheral != nil else { return }
    let old = connection
    ready = false; connection = nil; budget = 0; subscriber = nil
    sendTimer?.cancel(); sendTimer = nil; pendingIndication = nil
    let pending = sendInvoke; sendInvoke = nil; pending?.reject(code)
    // A fresh manager fences callbacks from an earlier central/connection. The
    // public owner and current share survive; no consumed-device tombstone.
    let manager = peripheral; peripheral = nil
    manager?.delegate = nil; manager?.stopAdvertising(); manager?.removeAllServices()
    serverChannel = nil; serverInformation = nil; serverService = nil
    if let old = old { event("disconnected", ["connection": old, "error": code]) }
    guard live else { retire("BLE_BACKGROUND", report: true); return }
    peripheral = CBPeripheralManager(delegate: self, queue: .main, options: [CBPeripheralManagerOptionShowPowerAlertKey: true])
  }
  func retire(_ code: String, report: Bool) {
    guard !retired else { return }
    if report {
      var fields: JSObject = ["error": code]
      if let connection = connection { fields["connection"] = connection }
      event(connection == nil ? "failed" : "disconnected", fields)
    }
    retired = true
    idleTimer.release()
    promptTimer?.cancel(); connectTimer?.cancel(); sendTimer?.cancel()
    promptTimer = nil; connectTimer = nil; sendTimer = nil
    let pending = [startInvoke, connectInvoke, sendInvoke]
    startInvoke = nil; connectInvoke = nil; sendInvoke = nil; pendingIndication = nil
    pending.forEach { $0?.reject(code) }
    central?.stopScan()
    if let selected = selected { selected.delegate = nil; central?.cancelPeripheralConnection(selected) }
    central?.delegate = nil; central = nil
    clientDelegate = nil
    peripheral?.stopAdvertising(); peripheral?.removeAllServices(); peripheral?.delegate = nil; peripheral = nil
    selected = nil; subscriber = nil; peers.removeAll(); seenSources.removeAll(); channel = nil; information = nil
    unnamedSources.removeAll()
    serverChannel = nil; serverInformation = nil; serverService = nil; receivedInfo = nil
    ready = false; connection = nil; budget = 0
    ended(self)
  }
}

private final class NearbyBluetoothConnectionDelegate: NSObject, CBPeripheralDelegate {
  private weak var session: NearbyBluetoothSession?
  private let handle: String
  init(session: NearbyBluetoothSession, handle: String) { self.session = session; self.handle = handle }
  private func current(_ candidate: CBPeripheral) -> NearbyBluetoothSession? {
    guard let session = session, session.ownsConnection(candidate, handle: handle) else { return nil }
    return session
  }
  func peripheral(_ candidate: CBPeripheral, didDiscoverServices error: Error?) {
    current(candidate)?.peripheral(candidate, didDiscoverServices: error)
  }
  func peripheral(_ candidate: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
    current(candidate)?.peripheral(candidate, didDiscoverCharacteristicsFor: service, error: error)
  }
  func peripheral(_ candidate: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
    current(candidate)?.peripheral(candidate, didUpdateValueFor: characteristic, error: error)
  }
  func peripheral(_ candidate: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
    current(candidate)?.peripheral(candidate, didUpdateNotificationStateFor: characteristic, error: error)
  }
  func peripheral(_ candidate: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
    current(candidate)?.peripheral(candidate, didWriteValueFor: characteristic, error: error)
  }
}
