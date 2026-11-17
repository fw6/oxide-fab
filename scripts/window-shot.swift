// Capture one window of a running app, by owner-name substring.
//
//   swift scripts/window-shot.swift oxide-fab artifacts/app.png
//
// Uses CGWindowList so the window does not have to be frontmost, which keeps
// CI (and a shared desktop) undisturbed.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count >= 3 else {
    FileHandle.standardError.write(Data("usage: window-shot.swift <owner> <out.png>\n".utf8))
    exit(2)
}
let owner = args[1]
let output = args[2]
let list = CGWindowListCopyWindowInfo([.optionAll, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
let candidates = list.filter { window in
    let name = window[kCGWindowOwnerName as String] as? String ?? ""
    let layer = window[kCGWindowLayer as String] as? Int ?? -1
    return name.contains(owner) && layer == 0
}
let pick = candidates.max { a, b in
    let area = { (w: [String: Any]) -> Int in
        let bounds = w[kCGWindowBounds as String] as? [String: Any] ?? [:]
        return (bounds["Width"] as? Int ?? 0) * (bounds["Height"] as? Int ?? 0)
    }
    return area(a) < area(b)
}
guard let window = pick, let number = window[kCGWindowNumber as String] as? Int else {
    FileHandle.standardError.write(Data("no window of \(owner)\n".utf8))
    exit(1)
}
let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
process.arguments = ["-x", "-l", String(number), output]
try process.run()
process.waitUntilExit()
if process.terminationStatus != 0 {
    exit(process.terminationStatus)
}
print("captured window \(number) to \(output)")
