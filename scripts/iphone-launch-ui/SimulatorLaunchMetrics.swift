import XCTest

@MainActor
final class SimulatorLaunchMetrics: XCTestCase {
    private var actionDeadline: TimeInterval = 0

    override func setUpWithError() throws {
        continueAfterFailure = false
        actionDeadline = try XCTUnwrap(
            ProcessInfo.processInfo.environment["KIWIX_LAUNCH_ACTION_DEADLINE"].flatMap(Double.init),
            "An absolute lifecycle deadline is required"
        )
        XCTAssertGreaterThan(actionDeadline, Date().timeIntervalSince1970)
    }

    private func withinActionWindow() -> Bool {
        guard Date().timeIntervalSince1970 < actionDeadline else {
            XCTFail("Absolute launch profiling action deadline reached")
            return false
        }
        return true
    }

    func test01OursLaunch() {
        let app = XCUIApplication(bundleIdentifier: "com.tiltastech.zimreader")
        let options = XCTMeasureOptions()
        options.iterationCount = 3
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: true)], options: options) {
            guard self.withinActionWindow() else { return }
            app.terminate()
            XCTAssertEqual(app.state, .notRunning, "Each launch must start with the application process stopped")
            guard self.withinActionWindow() else { return }
            app.launch()
            guard self.withinActionWindow() else { return }
            app.terminate()
        }
    }

    func test02UpstreamLaunch() {
        let app = XCUIApplication(bundleIdentifier: "com.tiltastech.zimreader.comparison.upstream")
        let options = XCTMeasureOptions()
        options.iterationCount = 3
        measure(metrics: [XCTApplicationLaunchMetric(waitUntilResponsive: true)], options: options) {
            guard self.withinActionWindow() else { return }
            app.terminate()
            XCTAssertEqual(app.state, .notRunning, "Each launch must start with the application process stopped")
            guard self.withinActionWindow() else { return }
            app.launch()
            guard self.withinActionWindow() else { return }
            app.terminate()
        }
    }
}
