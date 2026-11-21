import AppKit
import CoreGraphics

struct SmokeError: LocalizedError {
    let message: String
    var errorDescription: String? { message }

    init(_ message: String) {
        self.message = message
    }
}

func hasWindow(_ pid: pid_t) -> Bool {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
    return windows.contains {
        $0[kCGWindowOwnerPID as String] as? pid_t == pid &&
        $0[kCGWindowLayer as String] as? Int == 0
    }
}

func waitFor(_ condition: () -> Bool) -> Bool {
    let deadline = ProcessInfo.processInfo.systemUptime + 10
    while ProcessInfo.processInfo.systemUptime < deadline {
        if condition() {
            return true
        }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05))
    }
    return false
}

func smokeTest(_ app: URL) throws {
    guard let executable = Bundle(url: app)?.executableURL else {
        throw SmokeError("No executable in \(app.path).")
    }

    // Keep the path short enough for Perch's Unix-domain socket.
    let profile = URL(fileURLWithPath: "/tmp/perch-smoke-\(UUID().uuidString.prefix(12))")
    try FileManager.default.createDirectory(at: profile, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: profile) }

    let process = Process()
    process.executableURL = executable
    var environment = ProcessInfo.processInfo.environment
    environment["HOME"] = profile.path
    process.environment = environment
    try process.run()
    defer {
        if process.isRunning {
            process.terminate()
            process.waitUntilExit()
        }
    }

    guard waitFor({ !process.isRunning || hasWindow(process.processIdentifier) }),
          process.isRunning else {
        throw SmokeError("Perch did not open a window.")
    }
    print("Perch opened a window.")

    guard let application = NSRunningApplication(processIdentifier: process.processIdentifier),
          application.terminate() else {
        throw SmokeError("Could not send Quit to the test instance.")
    }
    guard waitFor({ !process.isRunning }), process.terminationStatus == 0 else {
        throw SmokeError("Perch did not quit successfully.")
    }

    let settings = profile.appendingPathComponent("Library/Application Support/perch/settings.json")
    guard let data = try? Data(contentsOf: settings),
          let document = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          let window = document["window"] as? [String: Any],
          let width = window["width"] as? Double, width > 0 else {
        throw SmokeError("Perch quit without saving window placement.")
    }
    print("Perch saved window placement and quit.")
}

do {
    guard CommandLine.arguments.count == 2 else {
        throw SmokeError("Usage: xcrun swift packaging/macos/smoke.swift <perch.app>")
    }
    try smokeTest(URL(fileURLWithPath: CommandLine.arguments[1]))
} catch {
    fputs("smoke: \(error.localizedDescription)\n", stderr)
    exit(1)
}
