import XCTest

// One session, real XCTest taps/keyboard/gestures; never injected DOM or state.
@MainActor
final class PhoneWindowUI: XCTestCase {
    private let environment = ProcessInfo.processInfo.environment
    private var app: XCUIApplication!
    private var productRole: String?

    private var appleLAN: Bool { environment["KIWIX_NEARBY_TOPOLOGY"] == "apple-lan" }

    private func prepareNearbyNetwork() throws {
        if appleLAN {
            // Native Apple transport can use available LAN/P2P connectivity.
            // No current snapshot distinguishes infrastructure from AWDL.
            logLine("KIWIX_NETWORK_PRECONDITION apple-lan; actual-interface=unmeasured")
        } else {
            try prepareRouterlessNetwork()
        }
    }

    override func setUpWithError() throws {
        continueAfterFailure = false
        guard let device = environment["KIWIX_DEVICE"], !device.isEmpty,
              environment["KIWIX_PHONE_WINDOW"] == "START-\(device)",
              let bundle = environment["KIWIX_APP_ID"], !bundle.isEmpty else {
            throw NSError(domain: "PhoneWindowAuthorization", code: 1)
        }
        if ["comparison", "nearby"].contains(environment["KIWIX_MODE"] ?? "") {
            executionTimeAllowance = try budget()
        } else {
            executionTimeAllowance = 300
        }
        app = XCUIApplication(bundleIdentifier: bundle)
        marker("started")
    }

    private func budget() throws -> TimeInterval {
        guard let raw = environment["KIWIX_DEADLINE"], let deadline = Double(raw) else {
            throw NSError(domain: "MissingDeadline", code: 1)
        }
        let remaining = deadline - Date().timeIntervalSince1970 - 3
        guard remaining > 0 else { throw NSError(domain: "PhoneWindowDeadline", code: 1) }
        return remaining
    }

    private func scoped(_ value: String) -> String {
        productRole.map { "\($0)/\(value)" } ?? value
    }

    private func logLine(_ value: String) {
        print(value.replacingOccurrences(of: "\n", with: "\\n").replacingOccurrences(of: "\r", with: "\\r"))
        fflush(stdout)
    }


    private func marker(_ phase: String) {
        print("KIWIX_WINDOW \(scoped(phase)) \(Date().timeIntervalSince1970)")
        fflush(stdout)
    }

    private func capture(_ phase: String) throws {
        _ = try budget()
        let image = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        image.name = "\(scoped(phase))-screen"
        image.lifetime = .keepAlways
        add(image)
        if phase == "unexpected-state" {
            // Full article trees are large and expensive to serialize. Preserve
            // screenshots at every checkpoint; collect the tree only on failure.
            _ = try budget()
            let tree = XCTAttachment(string: app.debugDescription)
            tree.name = "\(scoped(phase))-hierarchy"
            tree.lifetime = .keepAlways
            add(tree)
        }
        marker(phase)
    }

    private func reject(_ message: String) throws -> Never {
        logLine("KIWIX_FAILURE \(scoped(message))")
        // Capture the actual failing surface once, while still inside the action
        // deadline. The host permits only bounded result-file finalization next.
        do {
            try capture("unexpected-state")
        } catch {
            print("KIWIX_DIAGNOSTIC unavailable: \(error)")
            fflush(stdout)
        }
        XCTFail(message)
        throw NSError(domain: "UnexpectedUI", code: 1,
                      userInfo: [NSLocalizedDescriptionKey: message])
    }

    /// Select a retained reader tab only when its exact controls resolve.
    /// A missing or non-hit-testable switcher must fall back to the Library
    /// card; the article-content assertion below remains the authority.
    private func selectRetainedTab(_ title: String) -> Bool {
        let trigger = app.buttons.matching(NSPredicate(format:
            "label == %@ OR identifier == %@", "Open reader tabs", "Open reader tabs"))
        let usableTrigger = trigger.allElementsBoundByAccessibilityElement.filter { usable($0) }
        guard usableTrigger.count == 1 else { return false }
        usableTrigger[0].tap()
        let strip = app.otherElements.matching(NSPredicate(format:
            "label == %@ OR label == %@", "Open reader tabs", "Open reader tabs, navigation"))
        guard strip.count == 1 else { return false }
        let row = strip.firstMatch.buttons.matching(NSPredicate(format: "label == %@", "Close \(title)"))
        let tabs = strip.firstMatch.buttons.matching(NSPredicate(format: "label == %@", title))
        let usableTab = tabs.allElementsBoundByAccessibilityElement.filter { usable($0) }
        guard row.count == 1, usableTab.count == 1 else { return false }
        usableTab[0].tap()
        return true
    }

    /// Fixture flows require the retained tab; the Nearby sender may fall back.
    private func selectFixtureTab(_ title: String) throws {
        guard selectRetainedTab(title) else {
            try reject("Expected uniquely usable retained reader tab: \(title)")
        }
    }

    private func usable(_ element: XCUIElement) -> Bool {
        guard let snapshot = try? element.snapshot() else { return false }
        let frame = snapshot.frame
        // Native accessibility snapshots can include zero-sized WebViews with infinite origins.
        // Asking XCTest to hit-test those records an error instead of false.
        guard frame.origin.x.isFinite, frame.origin.y.isFinite,
              frame.width.isFinite, frame.height.isFinite,
              frame.width > 0, frame.height > 0 else { return false }
        // Hittability is not stable for remote/nested elements on every OS
        // build; a visible, enabled, finite control is the admission here and
        // taps fall back to a frame coordinate.
        return snapshot.isEnabled
    }

    private func one(_ query: XCUIElementQuery, _ description: String,
                     timeout: TimeInterval = 30,
                     accepting: @escaping (XCUIElement) -> Bool = { _ in true }) throws -> XCUIElement {
        logLine("KIWIX_STAGE \(scoped(description))")
        let remaining = try budget()
        var selected: XCUIElement?
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard (try? self.budget()) != nil else { return false }
            // Bind to accessibility identities, not indices that can move while
            // the page loads. Reject structural non-matches before remote
            // geometry/hit-point queries, and retain this one evaluated result.
            let matches = query.allElementsBoundByAccessibilityElement.filter {
                accepting($0) && self.usable($0)
            }
            selected = matches.count == 1 ? matches[0] : nil
            return selected != nil
        }, object: nil)
        guard XCTWaiter.wait(for: [expectation], timeout: min(timeout, remaining)) == .completed else {
            try reject("Expected exactly one hittable \(description)")
        }
        _ = try budget()
        guard let selected else { try reject("Element became unavailable: \(description)") }
        return selected
    }

    private func firstVisible(_ query: XCUIElementQuery, _ description: String,
                              timeout: TimeInterval = 5,
                              accepting: @escaping (XCUIElement) -> Bool = { _ in true }) throws -> XCUIElement {
        logLine("KIWIX_STAGE \(scoped(description))")
        let remaining = try budget()
        var selected: XCUIElement?
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard (try? self.budget()) != nil else { return false }
            selected = query.allElementsBoundByAccessibilityElement.first {
                accepting($0) && self.usable($0)
            }
            return selected != nil
        }, object: nil)
        guard XCTWaiter.wait(for: [expectation], timeout: min(timeout, remaining)) == .completed else {
            try reject("No visible \(description)")
        }
        _ = try budget()
        guard let selected else { try reject("Element became unavailable: \(description)") }
        return selected
    }

    /// Tap a visible control even when XCTest cannot hit-test it directly.
    @discardableResult
    private func tap(_ element: XCUIElement) -> XCUIElement {
        if (try? element.isHittable) == true {
            element.tap()
            return element
        }
        // Coordinate fallback is only licensed for a visible, enabled control
        // that no keyboard, sheet or alert frame covers at its center.
        guard finiteGeometry(element), element.isEnabled else {
            try? reject("Control is not visibly interactive: \(element)")
            return element
        }
        let frame = element.frame
        let center = CGPoint(x: frame.midX, y: frame.midY)
        let window = app.windows.firstMatch.frame
        let visible = frame.intersection(window)
        guard window.contains(center), !visible.isNull,
              visible.width * visible.height >= 0.5 * frame.width * frame.height else {
            try? reject("Control is clipped or off-screen: \(element)")
            return element
        }
        var covering: [XCUIElement] = []
        covering.append(contentsOf: app.keyboards.allElementsBoundByAccessibilityElement)
        covering.append(contentsOf: app.sheets.allElementsBoundByAccessibilityElement)
        covering.append(contentsOf: app.alerts.allElementsBoundByAccessibilityElement)
        if covering.contains(where: { $0.exists && $0.frame.contains(center) }) {
            try? reject("Control is obscured at its center: \(element)")
            return element
        }
        element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        return element
    }

    private func button(_ label: String) throws -> XCUIElement {
        try one(app.buttons.matching(NSPredicate(format: "label == %@ OR identifier == %@", label, label)), label)
    }

    private func animalResult(after field: XCUIElement) throws -> XCUIElement {
        let query = app.buttons.matching(NSPredicate(format:
            "label == %@ OR identifier == %@", "Animal", "Animal"))
        return try firstVisible(query, "Animal search result", timeout: 10,
                                accepting: { $0.frame.minY > field.frame.maxY })
    }

    /// The library is a scrolling list; a fixture card below the fold is not
    /// hittable. Scroll until the exact button is hittable, then let `one`
    /// report the precise failure if it never appears.
    private func scrollToButton(_ label: String) throws -> XCUIElement {
        let query = app.buttons.matching(NSPredicate(format: "label == %@ OR identifier == %@", label, label))
        for _ in 0..<10 {
            let matches = query.allElementsBoundByAccessibilityElement.filter { self.usable($0) }
            if matches.count == 1 { return matches[0] }
            if matches.count > 1 { break }
            app.swipeUp()
            _ = try budget()
        }
        return try button(label)
    }

    private func allowNearbyPrompts() throws {
        guard let app else { try reject("Application not initialized") }
        let owners = [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")]
        for owner in owners {
            for alert in owner.alerts.allElementsBoundByAccessibilityElement {
                guard alert.exists else { continue }
                let message = alert.label + " " + alert.staticTexts.allElementsBoundByAccessibilityElement.map(\.label).joined(separator: " ")
                guard message.localizedCaseInsensitiveContains("Kiwix") else { continue }
                for label in ["Allow", "Allow While Using App", "Allow Once", "OK"] {
                    let allow = alert.buttons[label]
                    if allow.exists && allow.isHittable {
                        allow.tap()
                        Thread.sleep(forTimeInterval: 0.5)
                        break
                    }
                }
            }
        }
    }

    private func scrollIntoVisibleArea(_ element: XCUIElement, maxSwipes: Int = 12) throws -> XCUIElement {
        guard let app else { try reject("Application not initialized") }
        let window = app.windows.firstMatch.frame
        let nav = app.buttons.matching(NSPredicate(format: "label == %@", "Library")).firstMatch
        let navAtBottom = nav.exists && finiteGeometry(nav) && nav.frame.minY > window.midY
        let bottom = navAtBottom ? nav.frame.minY : window.maxY - 20
        let visible = CGRect(x: window.minX, y: window.minY,
                             width: window.width, height: max(0, bottom - window.minY))
        let content = app.webViews.firstMatch
        for _ in 0..<maxSwipes {
            if element.exists && finiteGeometry(element) {
                let frame = element.frame
                if frame.minY >= visible.minY && frame.maxY <= visible.maxY { return element }
            }
            if content.exists { content.swipeUp() } else { app.swipeUp() }
            Thread.sleep(forTimeInterval: 0.25)
            _ = try budget()
        }
        guard element.exists && finiteGeometry(element) else {
            try reject("Element disappeared before scrolling into view")
        }
        let frame = element.frame
        guard frame.minY >= visible.minY && frame.maxY <= visible.maxY else {
            try reject("Element did not scroll into view")
        }
        return element
    }

    private func searchBooksControl() throws -> XCUIElement {
        // The observed reader exposes no button-role match for this name.
        // Keep the exact name and uniqueness checks without assuming its role.
        try one(app.descendants(matching: .any).matching(NSPredicate(format:
            "label == %@ OR identifier == %@", "Search books", "Search books")), "Search books")
    }

    private func readerNavigationControl() throws -> XCUIElement {
        let named = NSPredicate(format: "label == %@", "Reader navigation")
        // WebKit exposes the dialog trigger and its containing toolbar as Other.
        // The actionable named leaf is unique; the toolbar is not the trigger.
        return try one(app.otherElements.matching(named), "Reader navigation trigger",
                       accepting: { $0.descendants(matching: .any).matching(named).count == 0 })
    }

    private func archiveWebView() throws -> XCUIElement {
        // XCTest exposes nested accessibility wrappers for one document.
        // Select its visible leaf, not an incidental wrapper count or label.
        // Map probes/drags require a genuinely hittable document, not merely
        // a visible leaf.
        try one(app.webViews, "visible archive document", accepting: {
            $0.descendants(matching: .webView).count == 0 && $0.isHittable
        })
    }

    /// Read-only document selection: the same unique leaf, without a hit point.
    /// Some OS releases refuse hit-testing a nested WKWebView even while the
    /// article is fully visible; text reading and app screenshots do not need one.
    private func visibleArchiveWebView(containing fragment: String? = nil) throws -> XCUIElement {
        logLine("KIWIX_STAGE \(scoped("visible archive document"))")
        let remaining = try budget()
        var selected: XCUIElement?
        let expectation = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard (try? self.budget()) != nil else { return false }
            let leaves = self.app.webViews.allElementsBoundByAccessibilityElement.filter {
                $0.descendants(matching: .webView).count == 0 && self.finiteGeometry($0)
            }
            // Single document: no content query needed.
            if leaves.count == 1 {
                selected = leaves[0]
                return true
            }
            guard let fragment else {
                selected = nil
                return false
            }
            // Several retained tabs keep a document webview each. Bind the one
            // that holds this run's expected content; each remote query is slow.
            let matches = leaves.filter {
                $0.descendants(matching: .staticText)
                    .matching(NSPredicate(format: "label CONTAINS[c] %@", fragment)).count > 0
            }
            selected = matches.count == 1 ? matches[0] : nil
            return selected != nil
        }, object: nil)
        guard XCTWaiter.wait(for: [expectation], timeout: min(120, remaining)) == .completed else {
            try reject("Expected exactly one visible archive document")
        }
        _ = try budget()
        guard let selected else { try reject("Element became unavailable: visible archive document") }
        return selected
    }

    /// Geometry only; WebKit's enabled flag and hit-testing are not document authority.
    private func finiteGeometry(_ element: XCUIElement) -> Bool {
        guard let snapshot = try? element.snapshot() else { return false }
        let frame = snapshot.frame
        return frame.origin.x.isFinite && frame.origin.y.isFinite
            && frame.width.isFinite && frame.height.isFinite
            && frame.width > 0 && frame.height > 0
    }

    private func text(_ fragment: String, in archive: XCUIElement, exact: Bool = false) throws {
        logLine("KIWIX_STAGE \(scoped("article body: \(fragment)"))")
        let content = archive.descendants(matching: .staticText)
            .matching(NSPredicate(format: exact ? "label == %@" : "label CONTAINS[c] %@", fragment)).firstMatch
        guard content.waitForExistence(timeout: min(30, try budget())) else {
            try reject("Actual article content missing: \(fragment)")
        }
    }

    private func timed(_ label: String, _ action: () throws -> Void) throws {
        _ = try budget()
        let start = ProcessInfo.processInfo.systemUptime
        try action()
        print("KIWIX_TIMING \(label) \(ProcessInfo.processInfo.systemUptime - start)")
        fflush(stdout)
    }

    private func activateProduct(_ role: String) throws {
        productRole = role
        let key = role == "ours" ? "KIWIX_APP_ID" : "KIWIX_BASELINE_ID"
        guard let identifier = environment[key], !identifier.isEmpty else {
            try reject("Missing comparison app identity")
        }
        app = XCUIApplication(bundleIdentifier: identifier)
        try timed("\(role)_activation_to_foreground_observation_seconds") {
            app.activate()
            guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
                try reject("Comparison application did not become foreground")
            }
        }
    }

    private func rendezvous(_ name: String, checkpoint: Bool,
                            whileWaiting: (() throws -> Void)? = nil) throws {
        guard let role = productRole,
              let nonce = environment["KIWIX_CHECKPOINT_NONCE"],
              nonce.range(of: "^[a-f0-9]{32}$", options: .regularExpression) != nil,
              let rawDeadline = environment["KIWIX_DEADLINE"], let deadline = Double(rawDeadline),
              let identifier = environment[role == "ours" ? "KIWIX_APP_ID" : "KIWIX_BASELINE_ID"],
              let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first else {
            try reject("Missing authorized runner-private rendezvous identity")
        }
        let phase = scoped(name)
        let slug = phase.replacingOccurrences(of: "/", with: "-")
        let ack = documents.appendingPathComponent("checkpoint-\(nonce)-\(slug).json")
        try FileManager.default.createDirectory(at: documents, withIntermediateDirectories: true)
        if checkpoint { try capture(name) }
        let kind = checkpoint ? "KIWIX_CHECKPOINT" : "KIWIX_REQUEST"
        logLine("\(kind) \(phase) \(nonce)")
        let localDeadline = Date().addingTimeInterval(min(name == "sender-ready" ? 240 : 90, try budget()))
        let ending = Data(("\"complete\":\"KIWIX_ACK_END_\(nonce)\"}\n").utf8)
        while Date() < localDeadline {
            _ = try budget()
            try whileWaiting?()
            guard app.state == .runningForeground else {
                try reject("Application left foreground during \(phase)")
            }
            logLine("KIWIX_FOREGROUND \(phase) \(nonce) \(identifier) \(Date().timeIntervalSince1970)")
            if FileManager.default.fileExists(atPath: ack.path) {
                let values = try ack.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
                guard values.isRegularFile == true, values.isSymbolicLink != true,
                      (values.fileSize ?? 4097) <= 4096 else {
                    try reject("Rendezvous receipt is not a bounded regular runner-owned file")
                }
                let data = try Data(contentsOf: ack)
                // A container copy may expose an incomplete file. Only the final
                // marker licenses parsing; no product UI action is retried here.
                if data.suffix(ending.count) == ending,
                   let decoded = try? JSONSerialization.jsonObject(with: data),
                   let value = decoded as? [String: Any] {
                    guard value["schema"] as? Int == 1,
                          value["nonce"] as? String == nonce,
                          value["phase"] as? String == phase, value["app_id"] as? String == identifier,
                          value["deadline"] as? Double == deadline,
                          value["complete"] as? String == "KIWIX_ACK_END_\(nonce)" else {
                        try reject("Stale or mismatched rendezvous receipt")
                    }
                    if let binding = environment["KIWIX_NEARBY_BINDING"],
                       value["nearby_binding"] as? String != binding {
                        try reject("Nearby acknowledgement does not match this device and frozen source")
                    }
                    if let failure = value["failure"] as? String {
                        try reject("Paired host stopped: \(failure)")
                    }
                    _ = try budget()
                    if environment["KIWIX_PLATFORM"] == "simulator" && !checkpoint {
                        guard let rawURLs = environment["KIWIX_REQUEST_URLS"],
                              let data = rawURLs.data(using: .utf8),
                              let expected = try JSONSerialization.jsonObject(with: data) as? [String: String],
                              let rawURL = value["payload_url"] as? String,
                              expected[phase] == rawURL,
                              ["wiki-file", "map-file", "wiki-reset", "map-reset"].contains(name),
                              let url = URL(string: rawURL),
                              (name.hasSuffix("-file") ? url.isFileURL : url.scheme == "zim") else {
                            try reject("Unauthorized simulator fixture/navigation URL")
                        }
                        // Public targeted OS URL delivery (iOS 16.4+), not a
                        // process argument, product state injection or default-app routing.
                        app.open(url)
                        _ = try budget()
                    } else if value["payload_url"] != nil {
                        try reject("Unexpected URL in checkpoint acknowledgement")
                    }
                    guard app.state == .runningForeground else {
                        try reject("Application left foreground before checkpoint acknowledgement")
                    }
                    return
                }
            }
            Thread.sleep(forTimeInterval: min(1, try budget()))
        }
        try reject("Host acknowledgement deadline expired at \(phase)")
    }

    private func prepareOurWiki() throws {
        if environment["KIWIX_WIKI_MODE"] == "fresh" {
            try rendezvous("wiki-file", checkpoint: false)
            guard let filename = environment["KIWIX_WIKI_FILENAME"] else {
                try reject("Missing owned Wikipedia filename")
            }
            let heading = "Add \(filename) to your library?"
            _ = try one(app.webViews.descendants(matching: .staticText)
                .matching(NSPredicate(format: "label == %@", heading)), heading)
            try capture("wiki-intake")
            try tap(button("Add to library"))
        }
        try tap(button("Library"))
        if let title = environment["KIWIX_WIKI_TAB_TITLE"] {
            try selectFixtureTab(title)
            if environment["KIWIX_OURS_WIKI_AT_HOME"] != "1" {
                try readerNavigationControl().tap()
                try tap(button("Archive home"))
            }
        } else {
            try scrollToButton("Open Wikipedia 100").tap()
        }
    }

    private func selectNativeTab(_ title: String) throws {
        try tap(button("Tabs Manager"))
        let switcher = try one(app.navigationBars.matching(identifier: "Tabs"), "native tab switcher")
        try tap(button(title))
        // Switching tabs can dismiss the presenting BrowserTab's sheet.
        // Selecting the already-current tab leaves it open; only then use Done.
        if !switcher.waitForNonExistence(timeout: min(3, try budget())) {
            try one(switcher.buttons.matching(identifier: "Done"), "native tab switcher Done").tap()
            guard switcher.waitForNonExistence(timeout: min(10, try budget())) else {
                try reject("Native tab switcher did not dismiss")
            }
        }
    }

    private func prepareNativeFixture(_ name: String) throws {
        let key = name == "wiki" ? "WIKI" : "MAP"
        if environment["KIWIX_BASELINE_\(key)_IMPORT"] == "1" {
            try rendezvous("\(name)-file", checkpoint: false)
        } else if let title = environment["KIWIX_BASELINE_\(key)_TAB_TITLE"] {
            try selectNativeTab(title)
            // Pinned upstream supports zim:// UUID main-page URLs through its
            // ordinary onOpenURL path, loading the selected tab, not injected state.
            try rendezvous("\(name)-reset", checkpoint: false)
        } else {
            try tap(button("Library"))
            try one(app.tabBars.buttons.matching(identifier: "Opened"), "native Opened library tab").tap()
            let title = name == "wiki" ? "Wikipedia 100" : "Hawaii"
            try one(app.buttons.matching(identifier: title), "native fixture library card").tap()
            try tap(button("Open Main Page"))
        }
        _ = try visibleArchiveWebView()
    }

    private func typeAnimal(_ field: XCUIElement) throws {
        _ = try budget()
        field.tap()
        let value = field.value as? String ?? ""
        if value == "Animal" {
            _ = try budget()
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: value.count))
        } else if !value.isEmpty && !["Search", "Search books", "Search your offline library"].contains(value) {
            try reject("Refuse to overwrite a non-benchmark query")
        }
        _ = try budget()
        field.typeText("Animal")
        guard field.value as? String == "Animal" else { try reject("Actual Animal keyboard input did not match") }
    }

    private func searchAnimal(_ role: String) throws {
        let field: XCUIElement
        if role == "ours" {
            try searchBooksControl().tap()
            let fields = app.webViews.descendants(matching: .textField).matching(NSPredicate(format:
                "label == %@ OR placeholderValue == %@", "Search your offline library", "Search books"))
            field = try one(fields, "offline article search field")
            try typeAnimal(field)
            try timed("ours_search_tap_to_result_observation_seconds") {
                try tap(button("Search"))
                _ = try animalResult(after: field)
            }
        } else {
            _ = try budget()
            if !app.searchFields["Search"].isHittable {
                _ = try budget()
                app.swipeDown() // One source-defined searchable navigation-drawer reveal.
            }
            field = try one(app.searchFields.matching(NSPredicate(format:
                "label == %@ OR placeholderValue == %@", "Search", "Search")), "native offline Search field")
            try timed("baseline_query_input_to_result_observation_seconds") {
                try typeAnimal(field)
                _ = try animalResult(after: field)
            }
        }
        try capture("animal-results")
        try timed("\(role)_result_tap_to_body_observation_seconds") {
            try tap(try animalResult(after: field))
            let view = try visibleArchiveWebView(containing: "Animal")
            // Reading text is not tapping a control: repeated words and
            // non-hit-testable heading children are legitimate article content.
            try text("Animal", in: view, exact: true)
            // Bold/link boundaries split the real introduction into AX nodes.
            try text("are multicellular,", in: view)
            try text("organisms belonging to the biological kingdom", in: view)
        }
    }

    private func prepareOurMap() throws {
        if environment["KIWIX_MAP_MODE"] == "fresh" {
            try rendezvous("map-file", checkpoint: false)
            guard let filename = environment["KIWIX_MAP_FILENAME"] else { try reject("Missing owned map filename") }
            let heading = "Add \(filename) to your library?"
            _ = try one(app.webViews.descendants(matching: .staticText)
                .matching(NSPredicate(format: "label == %@", heading)), heading, timeout: 30)
            try capture("map-intake")
            try tap(button("Add to library"))
        }
        if let title = environment["KIWIX_MAP_TAB_TITLE"] {
            try selectFixtureTab(title)
        } else {
            try tap(button("Library"))
            try tap(button("Open Hawaii"))
        }
        try readerNavigationControl().tap()
        try tap(button("Archive home"))
    }

    private func interactWithMap(_ role: String) throws {
        let map = try archiveWebView()
        try text("Hawaii", in: map)
        _ = try one(map.links.matching(NSPredicate(format: "label CONTAINS %@", "OpenStreetMap")),
                    "actual OpenStreetMap attribution")
        // Reloading the archive restores its persisted camera. Its own Home
        // action establishes the same configured center/zoom for every run.
        try one(map.buttons.matching(NSPredicate(format: "label == %@", "Show the whole map")),
                "archive map Home").tap()
        Thread.sleep(forTimeInterval: min(1, try budget())) // Archive's camera animation is 800 ms.
        try capture("map-home")
        try one(map.buttons.matching(NSPredicate(format: "label == %@", "Zoom in")), "archive map Zoom in").tap()
        Thread.sleep(forTimeInterval: min(0.75, try budget()))
        try capture("map-zoom")
        // Lift the southern islands into view after zoom, rather than moving
        // horizontally across empty ocean in the native reader's taller canvas.
        let start = map.coordinate(withNormalizedOffset: CGVector(dx: 0.65, dy: 0.75))
        let finish = map.coordinate(withNormalizedOffset: CGVector(dx: 0.35, dy: 0.20))
        _ = try budget()
        start.press(forDuration: 0.15, thenDragTo: finish)
        Thread.sleep(forTimeInterval: min(0.75, try budget()))
        try capture("map-pan")
    }

    private func comparison() throws {
        guard let order = environment["KIWIX_ORDER"], ["AB", "BA"].contains(order) else {
            try reject("Unknown paired comparison order")
        }
        let roles = order == "AB" ? ["ours", "baseline"] : ["baseline", "ours"]
        // Both Wiki-home checkpoints precede either product's search control.
        for role in roles {
            try activateProduct(role)
            if role == "ours" { try prepareOurWiki() } else { try prepareNativeFixture("wiki") }
            _ = try budget()
            guard app.alerts.count == 0 else { try reject("Unexpected alert at Wiki-home checkpoint") }
            try rendezvous("wiki-home", checkpoint: true)
        }
        for role in roles {
            try activateProduct(role)
            try searchAnimal(role)
            try rendezvous("animal-body", checkpoint: true)
        }
        for role in roles {
            try activateProduct(role)
            if role == "ours" { try prepareOurMap() } else { try prepareNativeFixture("map") }
            try interactWithMap(role)
            try rendezvous("map", checkpoint: true)
        }
        productRole = nil
        try capture("complete")
    }

    func testLaunch() throws {
        guard environment["KIWIX_MODE"] == "launch" else {
            try reject("Launch-only test requires its prepared launch workload")
        }
        _ = try budget()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Application did not become foreground")
        }
        _ = try one(app.webViews, "visible application WebView", accepting: {
            $0.descendants(matching: .webView).count == 0
        })
        guard app.state == .runningForeground else {
            try reject("Application left the foreground before launch capture")
        }
        try capture("launch-complete")
    }

    /// Exercise the shipped shell without fixtures: Library actions, Settings
    /// grouping/status, and the Nearby controls. This is a layout/interaction
    /// smoke, not archive readability or transfer acceptance.
    func testUiSmoke() throws {
        guard environment["KIWIX_MODE"] == "launch" else {
            try reject("UI smoke requires its prepared launch workload")
        }
        XCUIDevice.shared.orientation = .portrait
        _ = try budget()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Application did not become foreground")
        }
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        try dismissStaleJoinPrompts(springboard: springboard)

        try tap(button("Library"))
        try capture("library")
        let details = app.buttons.matching(NSPredicate(format:
            "label BEGINSWITH %@", "Details and actions for"))
        try tap(try firstVisible(details, "book details action", timeout: 10))
        try capture("library-details")
        try tap(button("Close"))

        try tap(button("Settings"))
        _ = try firstVisible(app.staticTexts.matching(NSPredicate(format:
            "label == %@ OR label == %@", "Changes are saved automatically.", "Preferences saved")),
            "autosave status", timeout: 10)
        try capture("settings")
        _ = try scrollIntoVisibleArea(app.staticTexts["Sharing & devices"].firstMatch, maxSwipes: 12)
        try capture("settings-sharing")

        try tap(button("Nearby"))
        let stopFinding = app.buttons.matching(NSPredicate(format:
            "label == %@ OR identifier == %@", "Stop finding", "Stop finding"))
        let promptDeadline = Date().addingTimeInterval(15)
        var stopFindingReady = false
        while Date() < promptDeadline {
            try allowNearbyPrompts()
            let matches = stopFinding.allElementsBoundByAccessibilityElement.filter { usable($0) }
            if matches.count > 1 { try reject("Stop finding is ambiguous") }
            if matches.count == 1 { stopFindingReady = true; break }
            Thread.sleep(forTimeInterval: min(0.3, try budget()))
        }
        guard stopFindingReady else { try reject("Stop finding did not become available") }
        try allowNearbyPrompts()
        for owner in [app!, springboard] {
            for alert in owner.alerts.allElementsBoundByAccessibilityElement {
                let message = alert.label + " " + alert.staticTexts.allElementsBoundByAccessibilityElement.map(\.label).joined(separator: " ")
                if message.localizedCaseInsensitiveContains("Kiwix") {
                    try reject("Nearby permission alert remains")
                }
            }
        }
        _ = try one(app.staticTexts.matching(NSPredicate(format:
            "label == %@", "Receive from a nearby device")), "Nearby receive heading", timeout: 10)
        try capture("nearby")
        _ = try scrollIntoVisibleArea(try scrollToButton("Stop all"), maxSwipes: 8)
        try capture("nearby-stop-all")
        try capture("complete")
    }

    /// Exercise an existing Hawaii map and the Downloads surface without
    /// staging fixtures or changing archives.
    func testContentSmoke() throws {
        guard environment["KIWIX_MODE"] == "launch" else {
            try reject("Content smoke requires its prepared launch workload")
        }
        XCUIDevice.shared.orientation = .portrait
        _ = try budget()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Application did not become foreground")
        }
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        try dismissStaleJoinPrompts(springboard: springboard)

        try tap(button("Maps"))
        _ = try one(app.staticTexts.matching(NSPredicate(format:
            "label == %@", "Browse map areas")), "Maps heading", timeout: 15)
        let mapRegion = app.buttons.matching(NSPredicate(format:
            "label == %@", "Get map")).firstMatch
        let mapError = app.staticTexts.matching(NSPredicate(format:
            "label CONTAINS[c] %@ OR label CONTAINS[c] %@",
            "session has expired", "Browsing session expired")).firstMatch
        if !mapRegion.waitForExistence(timeout: min(20, try budget())) {
            if mapError.exists { try reject("Maps catalog still reports a session error") }
            logLine("KIWIX_MAP_CATALOG_OFFLINE continuing with offline map coverage")
        } else if mapError.exists {
            try reject("Maps catalog still reports a session error")
        }
        try capture("maps-catalog")

        try tap(button("Library"))
        try scrollToButton("Open Hawaii").tap()
        let map = try archiveWebView()
        try readerNavigationControl().tap()
        try tap(button("Archive home"))
        try text("Hawaii", in: map)
        let attribution = map.links.matching(NSPredicate(format: "label CONTAINS %@", "OpenStreetMap"))
        _ = try one(attribution, "actual OpenStreetMap attribution")
        try one(map.buttons.matching(NSPredicate(format: "label == %@", "Show the whole map")),
                "archive map Home").tap()
        try capture("map-home")
        let zoom = map.buttons.matching(NSPredicate(format:
            "label == %@ OR identifier == %@", "Zoom in", "Zoom in"))
        try one(zoom, "archive map Zoom in").tap()
        Thread.sleep(forTimeInterval: min(0.75, try budget()))
        try capture("map-zoom")

        try tap(button("Downloads"))
        _ = try one(app.staticTexts.matching(NSPredicate(format:
            "label == %@", "Downloads")), "Downloads heading", timeout: 10)
        try capture("downloads")
        try capture("complete")
    }

    func testAcceptanceWindow() throws {
        if environment["KIWIX_MODE"] == "comparison" {
            try comparison()
            return
        }
        try timed("activation_to_library_button_observation_seconds") {
            app.activate()
            guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
                try reject("Application did not become foreground")
            }
            try tap(button("Library"))
            if environment["KIWIX_WIKI_TAB_TITLE"] == nil {
                // Observation is an existence check: the fixture card can be
                // below the fold, and scrolling is not part of this timing.
                let label = environment["KIWIX_WIKI_OPEN_LABEL"] ?? "Open Wikipedia 100"
                let query = app.buttons.matching(NSPredicate(format:
                    "label == %@ OR identifier == %@", label, label))
                let exists = XCTNSPredicateExpectation(predicate:
                    NSPredicate(format: "exists == true"), object: query.firstMatch)
                guard XCTWaiter.wait(for: [exists], timeout: min(30, try budget())) == .completed else {
                    try reject("Expected \(label) in the library")
                }
            }
        }
        try capture("library")
        if let title = environment["KIWIX_WIKI_TAB_TITLE"] {
            try selectFixtureTab(title)
        } else {
            try scrollToButton(environment["KIWIX_WIKI_OPEN_LABEL"] ?? "Open Wikipedia 100").tap()
        }
        try capture("wiki-open")
        try searchBooksControl().tap()
        let fields = app.webViews.descendants(matching: .textField).matching(NSPredicate(format:
            "label == %@ OR placeholderValue == %@", "Search your offline library", "Search books"))
        let field = try one(fields, "offline article search field")
        _ = try budget()
        field.tap()
        let value = field.value as? String ?? ""
        if value == "Animal" {
            _ = try budget()
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: value.count))
        } else if !value.isEmpty && !["Search", "Search books", "Search your offline library"].contains(value) {
            try reject("Refuse to replace a non-benchmark search query")
        }
        _ = try budget()
        field.typeText("Animal")
        XCTAssertEqual(field.value as? String, "Animal")
        try timed("search_tap_to_result_observation_seconds") {
            try tap(button("Search"))
            _ = try animalResult(after: field)
        }
        try capture("animal-results")
        try timed("result_tap_to_body_observation_seconds") {
            try tap(try animalResult(after: field))
            let article = try visibleArchiveWebView(containing: "Animal")
            try text("Animal", in: article, exact: true)
            try text("are multicellular,", in: article)
            try text("organisms belonging to the biological kingdom", in: article)
        }
        // Compact reading chrome probe. XCTest gestures on a web view can be
        // delivered as accessibility scrolls with no DOM touch events, and the
        // feature deliberately requires a genuine touch, so this records the
        // outcome instead of failing the window. The host log carries the
        // verdict; the screenshots show the actual chrome state.
        let panning = try archiveWebView()
        // The WebView's accessibility frame can stay full-window even when the
        // toolbar is removed, so the observable is the shell navigation, which
        // becomes inert and leaves the accessibility tree while controls hide.
        let library = app.buttons["Library"]
        let shellBefore = library.exists
        panning.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
            .press(forDuration: 0.05,
                   thenDragTo: panning.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2)))
        var hid = false
        let hideDeadline = Date().addingTimeInterval(6)
        while Date() < hideDeadline {
            if !library.exists { hid = true; break }
            Thread.sleep(forTimeInterval: min(0.1, try budget()))
        }
        print("KIWIX_PAN_PROBE hidden=\(hid) shellBefore=\(shellBefore)")
        fflush(stdout)
        try capture(hid ? "reading-pan-hidden" : "reading-pan-probe")
        if hid {
            panning.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
                .press(forDuration: 0.05,
                       thenDragTo: panning.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8)))
            let showDeadline = Date().addingTimeInterval(6)
            while Date() < showDeadline {
                if library.exists { break }
                Thread.sleep(forTimeInterval: min(0.1, try budget()))
            }
            print("KIWIX_PAN_PROBE restored=\(library.exists)")
            fflush(stdout)
        }
        try capture("reading-pan-restored")
        try capture("wiki-body")
        // Host captures memory and delivers the already-staged unique file URL.
        // This is an expected handoff wait, never an UI retry/debug loop.
        if let mode = environment["KIWIX_MAP_MODE"] {
            if mode == "fresh" {
                guard let filename = environment["KIWIX_MAP_FILENAME"] else {
                    try reject("Fresh map admission is missing its owned filename")
                }
                let heading = "Add \(filename) to your library?"
                _ = try one(app.webViews.descendants(matching: .staticText)
                    .matching(NSPredicate(format: "label == %@", heading)), heading, timeout: 40)
                try capture("map-intake")
                try tap(button("Add to library"))
            }
            if let title = environment["KIWIX_MAP_TAB_TITLE"] {
                try selectFixtureTab(title)
            } else {
                try tap(button("Library"))
                try tap(button("Open Hawaii"))
            }
            try readerNavigationControl().tap()
            try tap(button("Archive home"))
            // Require actual archive semantics, not a canvas size or a tab title.
            let map = try archiveWebView()
            try text("Hawaii", in: map)
            let attribution = map.links.matching(NSPredicate(format: "label CONTAINS %@", "OpenStreetMap"))
            _ = try one(attribution, "actual OpenStreetMap attribution")
            try one(map.buttons.matching(NSPredicate(format: "label == %@", "Show the whole map")),
                    "archive map Home").tap()
            Thread.sleep(forTimeInterval: min(1, try budget()))
            try capture("map-home")
            let zoom = map.buttons.matching(NSPredicate(format:
                "label == %@ OR identifier == %@", "Zoom in", "Zoom in"))
            try one(zoom, "archive map Zoom in").tap()
            Thread.sleep(forTimeInterval: min(0.75, try budget())) // Bounded settle, not a latency sample.
            try capture("map-zoom")
            // Relative gesture in actual semantically identified archive WebView.
            // Screenshots require human inspection of real tiles/changed geography.
            let start = map.coordinate(withNormalizedOffset: CGVector(dx: 0.65, dy: 0.75))
            let finish = map.coordinate(withNormalizedOffset: CGVector(dx: 0.35, dy: 0.20))
            _ = try budget()
            start.press(forDuration: 0.15, thenDragTo: finish)
            Thread.sleep(forTimeInterval: min(0.75, try budget()))
            try capture("map-pan")
        }
        // Never close existing or newly created reader tabs, delete archives, or reset settings.
        try capture("complete")
    }


    /// Prove a real Nearby transfer, import and readable archive, not a visible
    /// Browse button. The host selects an archive absent from this receiver.
    func testNearby() throws {
        guard let fileName = environment["KIWIX_NEARBY_FILE_NAME"], !fileName.isEmpty,
              let title = environment["KIWIX_NEARBY_TITLE"], !title.isEmpty else {
            try reject("Nearby acceptance requires an exact source file name and archive title")
        }
        let expectedText = environment["KIWIX_NEARBY_EXPECTED_TEXT"] ?? title
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let promptOwners: [XCUIApplication] = [springboard, app]
        var joinStillVisible = false
        var selectedSender = false
        func handlePrompts() throws {
            var sawJoin = false
            for owner in promptOwners {
                for alert in owner.alerts.allElementsBoundByAccessibilityElement {
                    let message = alert.label + " " + alert.staticTexts.allElementsBoundByAccessibilityElement.map(\.label).joined(separator: " ")
                    if message.localizedCaseInsensitiveContains("Unable to join") {
                        try reject("iOS could not join the sender's network")
                    }
                    let join = alert.buttons["Join"]
                    if join.exists && join.isHittable {
                        if appleLAN { try reject("Apple LAN flow refuses temporary-hotspot Wi-Fi Join") }
                        if !selectedSender { try reject("An unrelated Wi-Fi Join prompt was already open before selecting the sender") }
                        sawJoin = true
                        if !joinStillVisible {
                            joinStillVisible = true
                            join.tap()
                        }
                        break
                    }
                    if message.localizedCaseInsensitiveContains("Kiwix") {
                        let actions = message.localizedCaseInsensitiveContains("Bluetooth")
                            ? ["OK", "Allow"] : ["Allow", "Allow While Using App", "Allow Once"]
                        for label in actions {
                            let allow = alert.buttons[label]
                            if allow.exists && allow.isHittable { allow.tap(); break }
                        }
                    }
                }
            }
            if !sawJoin { joinStillVisible = false }
        }
        app.launchEnvironment["KIWIX_NEARBY_DEBUG"] = "1"
        app.launch()
        try dismissStaleJoinPrompts(springboard: XCUIApplication(bundleIdentifier: "com.apple.springboard"))
        try prepareNearbyNetwork()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Application did not become foreground")
        }
        try handlePrompts()
        try tap(button("Nearby"))
        let find = app.buttons.matching(NSPredicate(format: "label == 'Find nearby'"))
        if find.count == 1, usable(find.firstMatch) { find.firstMatch.tap() }
        let choose = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Choose'"))
        let expectedPeer = environment["KIWIX_NEARBY_PEER"] ?? "Android"
        let candidates = choose.matching(NSPredicate(format: "label CONTAINS[c] %@", expectedPeer))
        // The paired sender may still be staging/publishing; bound by the
        // window but leave room for the catalog and download.
        let discoveryDeadline = Date().addingTimeInterval(min(180, max(10, try budget() - 30)))
        var selected: XCUIElement?
        while Date() < discoveryDeadline {
            try handlePrompts()
            let matches = candidates.allElementsBoundByAccessibilityElement.filter { usable($0) }
            if matches.count > 1 { try reject("Nearby sender \(expectedPeer) is ambiguous") }
            if let candidate = matches.first { selected = candidate; break }
            Thread.sleep(forTimeInterval: min(0.5, try budget()))
        }
        guard let peer = selected else { try reject("Expected Nearby sender \(expectedPeer) did not appear") }
        try capture("nearby-found")
        selectedSender = true
        peer.tap()


        let filters = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Find archives"))
        let failures = app.staticTexts.matching(NSPredicate(format:
            "label CONTAINS[c] 'stopped responding' OR label CONTAINS[c] 'no longer nearby' OR label CONTAINS[c] 'BLE_' OR label CONTAINS[c] 'JOIN_'"))
        let catalogDeadline = Date().addingTimeInterval(min(150, try budget()))
        var catalogReady = false
        while Date() < catalogDeadline {
            try handlePrompts()
            if failures.count > 0 { try reject("Nearby connection failed: \(failures.firstMatch.label)") }
            catalogReady = filters.allElementsBoundByAccessibilityElement.contains {
                ($0.elementType == .textField || $0.elementType == .searchField) && usable($0)
            }
            if catalogReady { break }
            Thread.sleep(forTimeInterval: min(0.5, try budget()))
        }
        guard catalogReady else { try reject("Nearby did not load a shared-library catalog") }
        try capture("nearby-catalog")
        let filter = try one(filters, "shared-library archive filter", accepting: {
            $0.elementType == .textField || $0.elementType == .searchField
        })
        filter.tap()
        if let value = filter.value as? String, !value.isEmpty, value != "Find archives" {
            filter.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: value.count))
        }
        filter.typeText(fileName)
        let done = app.toolbars.buttons["Done"]
        if done.exists && usable(done) { done.tap() }
        let download = try scrollToButton("Download")
        let started = ProcessInfo.processInfo.systemUptime
        download.tap()
        let closes = app.buttons.matching(NSPredicate(format: "label == %@", "Close")).allElementsBoundByAccessibilityElement
        if let footer = closes.last, usable(footer) { footer.tap() }
        try tap(button("Downloads"))
        let cardQuery = try nearbyDownloadCard(fileName)
        logLine("KIWIX_STAGE started Nearby download \(title)")
        let completionDeadline = Date().addingTimeInterval(min(140, try budget() - 15))
        var opened: XCUIElement?
        var nextProgress = ProcessInfo.processInfo.systemUptime
        while Date() < completionDeadline {
            try handlePrompts()
            // SwipeRow remounts on every progress update. Query descendants
            // through the card query, never through a previously bound card,
            // or XCUITest can resolve a retired accessibility identity.
            if cardQuery.count > 1 { try reject("Selected Nearby filename has duplicate download cards") }
            guard cardQuery.count == 1 else {
                Thread.sleep(forTimeInterval: min(0.5, try budget()))
                continue
            }
            // Progress text ("8 minutes remaining") changes between separate
            // bindings. Read every volatile label from one immutable snapshot.
            guard let cardSnapshot = try? cardQuery.firstMatch.snapshot() else {
                Thread.sleep(forTimeInterval: min(0.5, try budget()))
                continue
            }
            let states = snapshotLabels(cardSnapshot)
            let stopped = states.contains { ["Failed", "Paused", "Cancelled", "Unavailable"].contains($0) }
            let recoveryLabels = ["Resume \(fileName)", "Retry archive import", "Review low storage warning"]
            if stopped || states.contains(where: recoveryLabels.contains) {
                try reject("Selected Nearby download failed: \(states.joined(separator: " | "))")
            }
            if states.contains("Open \(title)") {
                let completed = cardQuery.buttons.matching(NSPredicate(format: "label == %@", "Open \(title)"))
                if completed.count == 1 {
                    if usable(completed.firstMatch) { opened = completed.firstMatch; break }
                    cardQuery.firstMatch.swipeUp()
                }
            }
            if ProcessInfo.processInfo.systemUptime >= nextProgress {
                logLine("KIWIX_NEARBY_PROGRESS \(states.joined(separator: " | "))")
                let image = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
                image.name = "nearby-download-progress"
                image.lifetime = .keepAlways
                add(image)
                nextProgress = ProcessInfo.processInfo.systemUptime + 15
            }
            Thread.sleep(forTimeInterval: min(0.5, try budget()))
        }
        guard let open = opened else { try reject("Selected Nearby archive did not finish download and managed import within its UI action bound") }
        logLine("KIWIX_TIMING nearby_completion_seconds \(ProcessInfo.processInfo.systemUptime - started)")
        try capture("nearby-completed")
        open.tap()
        let article = try visibleArchiveWebView(containing: expectedText)
        try text(expectedText, in: article)
        try capture("nearby-readable")
    }

    /// Retire only this receiver's discovery and host-authorized new download.
    /// The host separately proves journal UUID/offer ownership and preservation.
    func testNearbyRetire() throws {
        guard environment["KIWIX_NEARBY_RETIRE"] == "1", appleLAN,
              let fileName = environment["KIWIX_NEARBY_FILE_NAME"], !fileName.isEmpty,
              !fileName.contains("/"), !fileName.contains("\\"),
              let peer = environment["KIWIX_NEARBY_PEER"], !peer.isEmpty,
              let title = environment["KIWIX_NEARBY_TITLE"], !title.isEmpty,
              let jobID = environment["KIWIX_NEARBY_RETIRE_JOB_ID"],
              jobID.isEmpty || UUID(uuidString: jobID) != nil,
              let nonce = environment["KIWIX_CHECKPOINT_NONCE"],
              nonce.range(of: "^[a-f0-9]{32}$", options: .regularExpression) != nil else {
            try reject("Missing exact authorized receiver retirement")
        }
        productRole = "ours"
        // activate() may launch a stopped application; refuse that transition.
        guard app.state != .notRunning, app.state != .unknown else {
            try reject("Receiver retirement will not relaunch a stopped application")
        }
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(10, try budget())) else {
            try reject("Receiver did not become foreground for scoped retirement")
        }
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        guard app.alerts.count == 0, springboard.alerts.count == 0 else {
            try reject("Receiver retirement refuses unexpected system alerts")
        }
        let dialogs = app.otherElements.matching(NSPredicate(format: "label ENDSWITH %@", ", web dialog"))
        let catalog = app.otherElements.matching(NSPredicate(format: "label == %@", peer + ", web dialog"))
        if dialogs.count > 0 {
            guard dialogs.count == 1, catalog.count == 1 else {
                try reject("Receiver retirement found an unowned or ambiguous dialog")
            }
            try one(catalog.buttons.matching(NSPredicate(format: "label == %@", "Close")),
                    "Close within the exact owned source catalog", timeout: 5).tap()
        }
        try tap(button("Nearby"))
        let stop = app.buttons.matching(NSPredicate(format: "label == %@", "Stop finding"))
        let find = app.buttons.matching(NSPredicate(format: "label == %@", "Find nearby"))
        guard stop.count <= 1, find.count <= 1 else {
            try reject("Receiver discovery controls are ambiguous")
        }
        if stop.count == 1 {
            try one(stop, "owned receiver Stop finding", timeout: 5).tap()
        }
        _ = try one(find, "receiver discovery is off", timeout: 10)
        guard stop.count == 0 else { try reject("Receiver discovery did not retire") }
        try tap(button("Downloads"))
        let cards = app.otherElements.matching(NSPredicate(format: "label == %@", fileName + ", article"))
        if jobID.isEmpty {
            guard cards.count == 0 else {
                try reject("Receiver retirement found a download without host job authorization")
            }
        } else {
            let card = try nearbyDownloadCard(fileName)
            var cancellationRequested = false
            var detailsOpened = false
            while true {
                _ = try budget()
                guard card.count == 1 else {
                    try reject("Authorized receiver download card is missing or ambiguous")
                }
                let labels = snapshotLabels(try card.firstMatch.snapshot())
                let terminal = labels.contains { ["Completed", "Cancelled", "Failed"].contains($0) }
                if terminal {
                    let drained = card.buttons.matching(NSPredicate(format: "label == %@", "Delete download"))
                    guard drained.count <= 1 else { try reject("Ambiguous owned terminal download control") }
                    if drained.count == 1, drained.firstMatch.isEnabled {
                        // Observe only. Enabled means cancel_download's await
                        // and finally completed, not merely a durable status write.
                        if labels.contains("Cancelled") {
                            guard cancellationRequested else {
                                try reject("Receiver job was already cancelled without this invocation's command-drain proof")
                            }
                            logLine("KIWIX_RECEIVER_CANCELLED \(nonce)")
                        }
                        break
                    }
                    if drained.count == 0, !detailsOpened {
                        try one(card.descendants(matching: .any).matching(NSPredicate(
                            format: "label == %@", "Details for \(fileName)")),
                            "owned receiver download details", timeout: 5, accepting: {
                                $0.elementType == .button || $0.elementType == .disclosureTriangle
                            }).tap()
                        detailsOpened = true
                    }
                } else if !cancellationRequested {
                    let action = "Cancel \(fileName)"
                    try one(card.buttons.matching(NSPredicate(format: "label == %@", action)),
                            "Cancel within the exact authorized receiver card", timeout: 5).tap()
                    let confirmation = app.otherElements.matching(NSPredicate(
                        format: "label == %@", action + ", web dialog"))
                    _ = try one(confirmation, "exact owned download cancellation dialog", timeout: 5)
                    guard dialogs.count == 1 else { try reject("Ambiguous receiver cancellation dialog") }
                    _ = try one(confirmation.staticTexts.matching(NSPredicate(
                        format: "label == %@", "Cancel the download of “\(fileName)”?")),
                        "exact owned download cancellation question", timeout: 5)
                    try one(confirmation.buttons.matching(NSPredicate(format: "label == %@", action)),
                            "Confirm cancellation of only the authorized receiver job", timeout: 5).tap()
                    cancellationRequested = true
                }
                Thread.sleep(forTimeInterval: min(0.25, try budget()))
            }
        }
        guard dialogs.count == 0, app.alerts.count == 0, springboard.alerts.count == 0 else {
            try reject("Receiver retirement left an unexpected dialog or alert")
        }
        try capture("receiver-retired")
        logLine("KIWIX_RECEIVER_RETIRED \(nonce)")
        productRole = nil
    }

    /// Flatten one accessibility snapshot without issuing further live queries;
    /// progress labels can otherwise disappear between separate bindings.
    private func snapshotLabels(_ snapshot: XCUIElementSnapshot) -> [String] {
        var labels: [String] = []
        var pending: [XCUIElementSnapshot] = [snapshot]
        while let node = pending.popLast() {
            if !node.label.isEmpty { labels.append(node.label) }
            pending.append(contentsOf: node.children)
        }
        return labels
    }

    private func nearbyDownloadCard(_ filename: String) throws -> XCUIElementQuery {
        // The bare filename labels a heading; its article wrapper owns the
        // status and Open button in the observed physical WebKit hierarchy.
        // Return the query, never a bound card: SwipeRow remounts on progress
        // updates, so a retained element can become a retired identity.
        let query = app.otherElements.matching(NSPredicate(format: "label == %@", filename + ", article"))
        for _ in 0..<10 {
            let cards = query.allElementsBoundByAccessibilityElement
            if cards.count > 1 { try reject("Ambiguous download cards for \(filename)") }
            if let card = cards.first, usable(card) { return query }
            app.swipeUp()
            _ = try budget()
        }
        _ = try one(query, "exact Nearby download card \(filename)", timeout: 10)
        return query
    }

    /// Clear only preexisting system status before any request in this run.
    /// New association failures still abort in handlePrompts; never retry Join.
    private func dismissStaleJoinPrompts(springboard: XCUIApplication) throws {
        for host in [app, springboard].compactMap({ $0 }) {
            for alert in host.alerts.allElementsBoundByAccessibilityElement {
                _ = try budget()
                let text = ([alert.label] + alert.staticTexts.allElementsBoundByAccessibilityElement.map { $0.label })
                    .joined(separator: " ")
                if text.hasPrefix("Unable to join the network") {
                    let ok = alert.buttons.matching(NSPredicate(format: "label == %@", "OK"))
                    guard ok.count == 1, usable(ok.firstMatch) else {
                        try reject("Preexisting Wi-Fi failure has no exact OK control")
                    }
                    try capture("preexisting-join-failure")
                    logLine("KIWIX_PREEXISTING_JOIN_FAILURE acknowledged before peer selection")
                    ok.firstMatch.tap()
                } else if text.contains("Kiwix"), text.contains("Wants to Join") {
                    let cancel = alert.buttons.matching(NSPredicate(format: "label == %@", "Cancel"))
                    guard cancel.count == 1, usable(cancel.firstMatch) else {
                        try reject("Preexisting Kiwix Join request has no exact Cancel control")
                    }
                    cancel.firstMatch.tap()
                } else {
                    continue
                }
                Thread.sleep(forTimeInterval: min(0.5, try budget()))
            }
        }
    }

    /// Start without a router, but leave the Wi-Fi radio available for the
    /// app-configured peer network. Control Center disconnects without forgetting
    /// saved networks or disabling Wi-Fi as the Settings switch would.
    /// Control Center layouts differ across user customization and OS builds;
    /// accept the exact identifier or the unique Wi-Fi-labelled toggle tile.
    private func controlCenterWiFi(_ springboard: XCUIApplication) throws -> XCUIElement {
        let deadline = Date().addingTimeInterval(min(10, try budget()))
        while Date() < deadline {
            let exact = springboard.buttons.matching(identifier: "wifi-button")
            if exact.count == 1 { return exact.firstMatch }
            let labelled = springboard.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Wi-Fi"))
            let tiles = labelled.allElementsBoundByAccessibilityElement.filter {
                self.usable($0) && !$0.identifier.contains("-round")
            }
            if tiles.count == 1 { return tiles[0] }
            Thread.sleep(forTimeInterval: 0.25)
        }
        try reject("No visible Control Center Wi-Fi tile")
    }

    private func prepareRouterlessNetwork() throws {
        if ProcessInfo.processInfo.environment["KIWIX_SKIP_NETWORK_PRECONDITION"] == "1" {
            logLine("KIWIX_NETWORK_PRECONDITION skipped")
            return
        }
        _ = try budget()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        XCUIDevice.shared.press(.home)
        Thread.sleep(forTimeInterval: min(0.5, try budget()))
        for alert in springboard.alerts.allElementsBoundByAccessibilityElement {
            let joinTitle = alert.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "Wants to Join Wi-Fi"))
            if joinTitle.count > 0 {
                let cancel = alert.buttons.matching(NSPredicate(format: "label == %@", "Cancel"))
                if cancel.count == 1, cancel.firstMatch.isHittable {
                    cancel.firstMatch.tap()
                    Thread.sleep(forTimeInterval: min(0.5, try budget()))
                }
            }
            if alert.textViews["Join This Apple TV?"].exists {
                let close = alert.buttons.matching(NSPredicate(format: "label == %@", "Close"))
                guard close.count == 1, close.firstMatch.isHittable else {
                    try reject("Observed Apple TV proximity card has no exact Close")
                }
                close.firstMatch.tap()
            }
        }
        let start = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.01))
        let end = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.95, dy: 0.6))
        start.press(forDuration: 0.1, thenDragTo: end)
        Thread.sleep(forTimeInterval: min(1, try budget()))
        guard springboard.otherElements["cc-root-folder-view"].waitForExistence(timeout: min(5, try budget())) else {
            try reject("Control Center did not open for the routerless precondition")
        }
        let wifi = try controlCenterWiFi(springboard)
        let airplane = try one(springboard.buttons.matching(identifier: "airplane-button"),
                               "visible Airplane Mode tile", timeout: 5)
        let bluetooth = try one(springboard.buttons.matching(identifier: "bluetooth-button"),
                                "visible Bluetooth tile", timeout: 5)
        guard airplane.value as? String == "Off", bluetooth.value as? String == "On" else {
            try reject("Routerless testing requires Airplane Mode off and Bluetooth on")
        }
        let disconnected = ["Not Connected", "Disconnected"]
        guard let initial = wifi.value as? String, initial != "Off" else {
            try reject("Wi-Fi radio is off rather than disconnected")
        }
        if !disconnected.contains(initial) {
            try one(wifi.buttons.matching(identifier: "wifi-button-round"),
                    "Wi-Fi disconnect control", timeout: 5).tap()
        }
        let condition = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard (try? self.budget()) != nil else { return false }
            return disconnected.contains(wifi.value as? String ?? "")
        }, object: nil)
        guard XCTWaiter.wait(for: [condition], timeout: min(5, try budget())) == .completed,
              bluetooth.value as? String == "On", airplane.value as? String == "Off" else {
            try reject("iPhone did not reach disconnected Wi-Fi with Bluetooth still on")
        }
        logLine("KIWIX_NETWORK_PRECONDITION wifi=\(wifi.value as? String ?? "") bluetooth=On airplane=Off")
        try capture("network-disconnected")
        let tree = XCTAttachment(string: springboard.debugDescription)
        tree.name = "network-disconnected-hierarchy"
        tree.lifetime = .keepAlways
        add(tree)
        XCUIDevice.shared.press(.home)
    }

    func testNearbyVerify() throws {
        guard environment["KIWIX_NEARBY_OPERATION"] == "verify-received",
              let filename = environment["KIWIX_NEARBY_FILE_NAME"],
              let title = environment["KIWIX_NEARBY_TITLE"],
              let expected = environment["KIWIX_NEARBY_EXPECTED_TEXT"] else {
            try reject("Missing previously received Nearby identity")
        }
        try dismissStaleJoinPrompts(springboard: XCUIApplication(bundleIdentifier: "com.apple.springboard"))
        try prepareNearbyNetwork()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Receiver did not become foreground for existing-job verification")
        }
        try tap(button("Downloads"))
        let cardQuery = try nearbyDownloadCard(filename)
        let query = cardQuery.buttons.matching(NSPredicate(format: "label == %@", "Open \(title)"))
        for _ in 0..<10 {
            if query.count == 1, usable(query.firstMatch) { break }
            cardQuery.firstMatch.swipeUp()
            _ = try budget()
        }
        let open = try one(query, "Open in the exact previously received archive card", timeout: 10)
        try capture("nearby-existing")
        open.tap()
        try text(expected, in: visibleArchiveWebView(containing: expected))
        try capture("nearby-readable")
    }

    /// Ordinary file intake followed by real foreground sharing. The runner's
    /// private acknowledgement is released only after the paired peer proves reception.
    func testNearbySend() throws {
        guard ["iphone-to-android", "iphone-to-mac", "iphone-to-iphone"].contains(environment["KIWIX_NEARBY_DIRECTION"] ?? ""),
              let filename = environment["KIWIX_NEARBY_STAGED_NAME"],
              let title = environment["KIWIX_NEARBY_TITLE"],
              let expectedText = environment["KIWIX_NEARBY_EXPECTED_TEXT"],
              let nonce = environment["KIWIX_CHECKPOINT_NONCE"],
              let sourceMode = environment["KIWIX_NEARBY_SOURCE_MODE"],
              ["fresh", "existing"].contains(sourceMode),
              environment["KIWIX_NEARBY_BINDING"] != nil else {
            try reject("Missing authorized Nearby sender source")
        }
        productRole = "ours"
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Sender application did not become foreground")
        }
        try dismissStaleJoinPrompts(springboard: XCUIApplication(bundleIdentifier: "com.apple.springboard"))
        if sourceMode == "fresh" {
            try rendezvous("sender-file", checkpoint: false)
            let intake = app.otherElements.matching(NSPredicate(
                format: "label == %@", "Add \(filename) to your library?, web dialog"))
            _ = try one(intake, "exact owned Nearby source intake dialog")
            try capture("sender-intake")
            let storageName = NSPredicate(format: "label == %@", "Storage options")
            try one(intake.buttons.matching(storageName), "owned intake Storage options leaf",
                    accepting: { $0.buttons.matching(storageName).count == 0 }).tap()
            // A developer file URL grants no retained-original authority. Choose
            // this intake's ordinary managed copy without changing global settings.
            try one(intake.buttons.matching(NSPredicate(format: "label == %@", "Save a copy in Kiwix")),
                    "Save a copy in Kiwix for the owned Nearby source").tap()
        } else {
            try capture("sender-existing")
        }
        let retained = environment["KIWIX_NEARBY_SOURCE_TAB"].map(selectRetainedTab) ?? false
        if !retained {
            try tap(button("Library"))
            try scrollToButton("Open \(title)").tap()
        }
        let article = try visibleArchiveWebView(containing: expectedText)
        let sourceContent = article.descendants(matching: .any).matching(NSPredicate(
            format: "(elementType == %d OR elementType == %d) AND label CONTAINS[c] %@",
            XCUIElement.ElementType.staticText.rawValue, XCUIElement.ElementType.link.rawValue, expectedText))
        guard sourceContent.firstMatch.waitForExistence(timeout: min(30, try budget())) else {
            try reject("Actual sender archive content missing: \(expectedText)")
        }
        try rendezvous("sender-imported", checkpoint: true)
        // Verify the source before leaving the reader: this iOS version can
        // omit an existing iframe's accessibility subtree after SpringBoard.
        // Neither publisher nor connection starts before this precondition.
        try prepareNearbyNetwork()
        app.activate()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Sender did not return from Nearby network setup")
        }
        try tap(button("Nearby"))
        // Bind this run's advertisement to a unique name so a cached older
        // session under the platform default name cannot make the receiver
        // ambiguous. This is the product's ordinary device-name action.
        if let sharing = environment["KIWIX_NEARBY_SHARING_NAME"], !sharing.isEmpty {
            let section = app.buttons.matching(NSPredicate(format: "label == %@", "Device name"))
            if section.count == 1, usable(section.firstMatch) { tap(section.firstMatch) }
            let field = app.textFields.matching(NSPredicate(format:
                "label CONTAINS %@ OR placeholderValue == %@", "Nearby device name", "Automatic platform name"))
            guard field.count == 1, usable(field.firstMatch) else {
                try reject("Nearby device-name field missing for the run name")
            }
            if field.count == 1, usable(field.firstMatch) {
                let input = field.firstMatch
                input.tap()
                if let current = input.value as? String, !current.isEmpty,
                   current != "Automatic platform name" {
                    input.press(forDuration: 1.0)
                    let selectAll = app.menuItems["Select All"]
                    if selectAll.waitForExistence(timeout: 2) { selectAll.tap() }
                }
                input.typeText(sharing)
                // Submit through the form so editing ends and the keyboard
                // releases; otherwise it covers the Share all control.
                input.typeText("\n")
                let save = app.buttons.matching(NSPredicate(format: "label == %@", "Save device name"))
                if save.count == 1, usable(save.firstMatch) { tap(save.firstMatch) }
                if app.keyboards.count > 0 { input.typeText("\n") }
                app.swipeDown()
                app.swipeDown()
                _ = try one(app.buttons.matching(NSPredicate(format: "label == %@", "Device name")),
                            "Device name section after saving the run name")
                try capture("sender-named")
            }
        }
        // A previous aborted window can leave this device advertising an old
        // session under the same peer name, which makes the receiver's cards
        // ambiguous. Retract that stale session before publishing again.
        // Unique per-run names already disambiguate cached sessions; retracting
        // an old publication here would churn the identity a paired observer
        // has bound. Retain the stop only for legacy unnamed runs.
        if environment["KIWIX_NEARBY_SHARING_NAME"] == nil {
            let priorStop = app.otherElements.matching(NSPredicate(format: "label == %@ OR label == %@",
                "You’re sharing your library", "You’re sharing your library, region"))
                .buttons.matching(NSPredicate(format: "label == %@", "Stop sharing"))
            if priorStop.count == 1, usable(priorStop.firstMatch) {
                tap(priorStop.firstMatch)
                let idle = app.otherElements.matching(NSPredicate(format: "label == %@ OR label == %@",
                    "Share your library", "Share your library, region"))
                    .buttons.matching(NSPredicate(format: "label == %@", "Share all"))
                _ = try one(idle, "Idle library-sharing section after stopping a stale session")
                try capture("sender-stale-stopped")
            }
        }
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        var sharingRequested = false
        var joined = false
        var joinVisible = false
        func prompts() throws {
            var sawJoin = false
            for owner in [springboard, app!] {
                for alert in owner.alerts.allElementsBoundByAccessibilityElement {
                    let message = alert.label + " " + alert.staticTexts.allElementsBoundByAccessibilityElement.map(\.label).joined(separator: " ")
                    if message.localizedCaseInsensitiveContains("Unable to join") {
                        try reject("iOS sender could not join Android's reverse network")
                    }
                    let join = alert.buttons["Join"]
                    if join.exists && join.isHittable {
                        if appleLAN { try reject("Apple LAN flow refuses temporary-hotspot Wi-Fi Join") }
                        guard sharingRequested else { try reject("Unrelated Join prompt before sender sharing") }
                        sawJoin = true
                        if !joinVisible {
                            guard !joined else { try reject("Sender requested a second iOS Wi-Fi Join") }
                            joined = true
                            joinVisible = true
                            logLine("KIWIX_NEARBY_JOIN \(nonce) 1")
                            try capture("sender-join")
                            join.tap()
                        }
                    } else if message.localizedCaseInsensitiveContains("Kiwix") {
                        let actions = message.localizedCaseInsensitiveContains("Bluetooth")
                            ? ["OK", "Allow"] : ["Allow", "Allow While Using App", "Allow Once"]
                        for label in actions {
                            let allow = alert.buttons[label]
                            if allow.exists && allow.isHittable { allow.tap(); break }
                        }
                    } else {
                        try reject("Unexpected system alert during Nearby sending: \(message)")
                    }
                }
            }
            if !sawJoin { joinVisible = false }
        }
        try prompts()
        sharingRequested = true
        logLine("KIWIX_NEARBY_JOIN \(nonce) 0")
        let started = ProcessInfo.processInfo.systemUptime
        let share = app.otherElements.matching(NSPredicate(format: "label == %@ OR label == %@",
            "Share your library", "Share your library, region"))
            .buttons.matching(NSPredicate(format: "label == %@", "Share all"))
        try one(share, "Share all in the library-sharing section").tap()
        let stop = app.otherElements.matching(NSPredicate(format: "label == %@ OR label == %@",
            "You’re sharing your library", "You’re sharing your library, region"))
            .buttons.matching(NSPredicate(format: "label == %@", "Stop sharing"))
        var ownsSharing = true
        defer {
            if ownsSharing && stop.count == 1 && usable(stop.firstMatch) { stop.firstMatch.tap() }
        }
        let readyDeadline = Date().addingTimeInterval(min(30, try budget()))
        while Date() < readyDeadline {
            try prompts()
            if stop.count == 1, usable(stop.firstMatch) { break }
            Thread.sleep(forTimeInterval: min(0.5, try budget()))
        }
        guard stop.count == 1, usable(stop.firstMatch) else { try reject("Nearby Share all did not start") }
        try rendezvous("sender-ready", checkpoint: true, whileWaiting: prompts)
        logLine("KIWIX_NEARBY_JOIN \(nonce) \(joined ? 1 : 0)")
        logLine("KIWIX_TIMING nearby_completion_seconds \(ProcessInfo.processInfo.systemUptime - started)")
        stop.firstMatch.tap()
        _ = try one(share, "retired library publisher's Share all control")
        guard stop.count == 0 else { try reject("Library publisher is still active after Stop sharing") }
        ownsSharing = false
        try capture("sender-complete")
        productRole = nil
    }

    /// Cancelling a shared catalog must not leave a duplicate peer or a stuck
    /// action gate while the abandoned browse request is still winding down.
    func testNearbyCancel() throws {
        addUIInterruptionMonitor(withDescription: "Nearby permissions") { alert in
            for label in ["Allow", "OK", "Join", "Allow While Using App"] {
                let button = alert.buttons[label]
                if button.exists && button.isHittable { button.tap(); return true }
            }
            return false
        }
        app.launch()
        guard app.wait(for: .runningForeground, timeout: min(30, try budget())) else {
            try reject("Application did not become foreground")
        }
        try tap(button("Nearby"))
        let find = app.buttons.matching(NSPredicate(format: "label == 'Find nearby'"))
        if find.count > 0 && find.firstMatch.isHittable { find.firstMatch.tap() }
        app.tap()
        let choose = app.buttons.matching(NSPredicate(format: "label BEGINSWITH 'Choose'"))
        var peer: XCUIElement?
        let findDeadline = Date().addingTimeInterval(40)
        while Date() < findDeadline {
            if choose.count > 0, choose.firstMatch.exists, choose.firstMatch.isHittable, choose.firstMatch.isEnabled {
                peer = choose.firstMatch
                break
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        guard let peer else { try reject("No hittable nearby device appeared") }
        let before = choose.count
        peer.tap()
        let close = app.buttons.matching(NSPredicate(format: "label == 'Close'")).firstMatch
        guard close.waitForExistence(timeout: 10) else { try reject("Shared catalog did not open") }
        close.tap()
        let deadline = Date().addingTimeInterval(5)
        var stable = false
        while Date() < deadline {
            if choose.count <= before, choose.count > 0, choose.firstMatch.isEnabled {
                stable = true
                break
            }
            Thread.sleep(forTimeInterval: 0.25)
        }
        try capture("nearby-cancel")
        if !stable { try reject("Cancel left a duplicate or disabled peer list") }
    }
}
