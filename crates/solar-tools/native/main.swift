import AppKit
import ApplicationServices
import ScreenCaptureKit
import Foundation
import CoreMedia

enum NativeFailure: Error, LocalizedError {
    case message(String)
    var errorDescription: String? { if case let .message(value) = self { return value }; return nil }
}

@available(macOS 14.0, *)
@MainActor
final class NativeService: NSObject, SCContentSharingPickerObserver, SCStreamDelegate {
    private var filter: SCContentFilter?
    private var stream: SCStream?
    private let discardedFrames = DiscardedFrames()
    private var continuation: CheckedContinuation<SCContentFilter, Error>?
    private let picker = SCContentSharingPicker.shared

    override init() {
        super.init()
        picker.add(self)
    }

    nonisolated func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.filter = nil
            try? await self.stream?.stopCapture()
            self.stream = nil
            self.continuation?.resume(throwing: NativeFailure.message("Screen selection cancelled"))
            self.continuation = nil
        }
    }

    nonisolated func contentSharingPicker(_ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            if let stream {
                do { try await stream.updateContentFilter(filter) }
                catch { self.filter = nil; return }
            }
            self.filter = filter
            self.continuation?.resume(returning: filter)
            self.continuation = nil
        }
    }

    nonisolated func contentSharingPickerStartDidFailWithError(_ error: Error) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.continuation?.resume(throwing: error)
            self.continuation = nil
            self.filter = nil
        }
    }

    nonisolated func stream(_ stream: SCStream, didStopWithError error: Error) {
        Task { @MainActor [weak self] in
            guard let self else { return }
            if self.stream === stream { self.filter = nil; self.stream = nil }
        }
    }

    func handle(_ action: String, _ arguments: [String: Any]) async throws -> [String: Any] {
        switch action {
        case "permissions":
            return ["supported": true, "accessibility": AXIsProcessTrusted(), "screenRecording": CGPreflightScreenCaptureAccess(), "sharing": filter != nil]
        case "share_start":
            if let stream { try? await stream.stopCapture(); self.stream = nil }
            filter = nil
            NSApplication.shared.setActivationPolicy(.accessory)
            NSApplication.shared.activate(ignoringOtherApps: true)
            picker.isActive = true
            let selected = try await withCheckedThrowingContinuation { (pending: CheckedContinuation<SCContentFilter, Error>) in
                continuation = pending
                picker.present()
            }
            let configuration = SCStreamConfiguration()
            configuration.width = 640
            configuration.height = 480
            configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
            configuration.queueDepth = 1
            configuration.capturesAudio = false
            let stream = SCStream(filter: selected, configuration: configuration, delegate: self)
            try stream.addStreamOutput(discardedFrames, type: .screen, sampleHandlerQueue: DispatchQueue(label: "net.solar.discarded-frames"))
            do { try await stream.startCapture() }
            catch { filter = nil; picker.isActive = false; throw error }
            self.stream = stream
            return ["sharing": true, "contentRect": rect(selected.contentRect), "pointPixelScale": selected.pointPixelScale, "controlRequiresDisplay": true]
        case "share_stop":
            filter = nil
            picker.isActive = false
            if let stream { try? await stream.stopCapture(); self.stream = nil }
            return ["sharing": false]
        case "capture":
            guard let filter else { throw NativeFailure.message("Screen sharing is not active") }
            let config = SCStreamConfiguration()
            let scale = min(CGFloat(filter.pointPixelScale), 2048 / max(1, max(filter.contentRect.width, filter.contentRect.height)))
            config.width = max(1, Int(filter.contentRect.width * scale))
            config.height = max(1, Int(filter.contentRect.height * scale))
            config.showsCursor = true
            let image = try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
            let representation = NSBitmapImageRep(cgImage: image)
            guard let png = representation.representation(using: .png, properties: [:]) else { throw NativeFailure.message("Could not encode screenshot") }
            guard png.count <= 12_000_000 else { throw NativeFailure.message("Screenshot exceeds the capture size limit") }
            return ["image": png.base64EncodedString(), "width": image.width, "height": image.height, "contentRect": rect(filter.contentRect), "pointPixelScale": filter.pointPixelScale]
        case "inspect":
            try requireControl()
            let system = AXUIElementCreateSystemWide()
            var application: CFTypeRef?
            guard AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute as CFString, &application) == .success, let application else { throw NativeFailure.message("No focused application") }
            let focusedApplication = unsafeBitCast(application, to: AXUIElement.self)
            var window: CFTypeRef?
            guard AXUIElementCopyAttributeValue(focusedApplication, kAXFocusedWindowAttribute as CFString, &window) == .success, let window else { throw NativeFailure.message("No focused window") }
            var remaining = 150
            return ["tree": inspect(unsafeBitCast(window, to: AXUIElement.self), depth: 0, remaining: &remaining)]
        case "click":
            try requireControl(requireFocusedWindow: false)
            guard let x = arguments["x"] as? Double, let y = arguments["y"] as? Double, x.isFinite, y.isFinite else { throw NativeFailure.message("Finite x and y coordinates are required") }
            let point = CGPoint(x: x, y: y)
            guard let filter, filter.contentRect.contains(point) else { throw NativeFailure.message("The click is outside the shared display") }
            let right = arguments["button"] as? String == "right"
            let button: CGMouseButton = right ? .right : .left
            CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: button)?.post(tap: .cghidEventTap)
            CGEvent(mouseEventSource: nil, mouseType: right ? .rightMouseDown : .leftMouseDown, mouseCursorPosition: point, mouseButton: button)?.post(tap: .cghidEventTap)
            CGEvent(mouseEventSource: nil, mouseType: right ? .rightMouseUp : .leftMouseUp, mouseCursorPosition: point, mouseButton: button)?.post(tap: .cghidEventTap)
            return ["clicked": true]
        case "type":
            try requireControl()
            guard let text = arguments["text"] as? String, text.utf16.count <= 20_000 else { throw NativeFailure.message("Text is missing or too long") }
            let units = Array(text.utf16)
            var start = 0
            while start < units.count {
                var end = min(start + 20, units.count)
                if end < units.count && (0xD800...0xDBFF).contains(units[end - 1]) { end -= 1 }
                var chunk = Array(units[start..<end])
                guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false) else { throw NativeFailure.message("Keyboard events unavailable") }
                down.keyboardSetUnicodeString(stringLength: chunk.count, unicodeString: &chunk)
                up.keyboardSetUnicodeString(stringLength: chunk.count, unicodeString: &chunk)
                down.flags = []; up.flags = []
                down.post(tap: .cghidEventTap)
                up.post(tap: .cghidEventTap)
                start = end
            }
            return ["typed": true]
        case "key":
            try requireControl()
            let parts = (arguments["key"] as? String ?? "").uppercased().split(separator: "+").map(String.init)
            let codes: [String: CGKeyCode] = ["A":0,"S":1,"D":2,"F":3,"H":4,"G":5,"Z":6,"X":7,"C":8,"V":9,"B":11,"Q":12,"W":13,"E":14,"R":15,"Y":16,"T":17,"O":31,"U":32,"I":34,"P":35,"L":37,"J":38,"K":40,"N":45,"M":46,"RETURN":36,"ENTER":36,"TAB":48,"SPACE":49,"DELETE":51,"BACKSPACE":51,"ESCAPE":53,"ESC":53,"LEFT":123,"RIGHT":124,"DOWN":125,"UP":126]
            guard let key = parts.last, let code = codes[key] else { throw NativeFailure.message("Unsupported keyboard key") }
            var flags: CGEventFlags = []
            for modifier in parts.dropLast() {
                switch modifier { case "CMD", "COMMAND": flags.insert(.maskCommand); case "SHIFT": flags.insert(.maskShift); case "ALT", "OPTION": flags.insert(.maskAlternate); case "CTRL", "CONTROL": flags.insert(.maskControl); default: throw NativeFailure.message("Unsupported modifier") }
            }
            let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true)
            let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false)
            down?.flags = flags; up?.flags = flags
            down?.post(tap: .cghidEventTap); up?.post(tap: .cghidEventTap)
            return ["pressed": true]
        default: throw NativeFailure.message("Unknown native operation")
        }
    }

    private func requireAccessibility() throws {
        let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
        guard AXIsProcessTrustedWithOptions(options) else { throw NativeFailure.message("Enable Solar Accessibility access in System Settings, then retry") }
    }

    private func requireControl(requireFocusedWindow: Bool = true) throws {
        guard let filter, stream != nil else { throw NativeFailure.message("Screen sharing is not active") }
        if #available(macOS 14.2, *) {
            guard filter.style == .display else { throw NativeFailure.message("Native computer control requires sharing a complete display. Window/app sharing supports observation only.") }
        } else {
            throw NativeFailure.message("Native computer control requires macOS 14.2 or newer; screen observation remains available")
        }
        try requireAccessibility()
        if !requireFocusedWindow { return }
        let system = AXUIElementCreateSystemWide()
        var application: CFTypeRef?
        guard AXUIElementCopyAttributeValue(system, kAXFocusedApplicationAttribute as CFString, &application) == .success, let application else { throw NativeFailure.message("No focused application") }
        var window: CFTypeRef?
        guard AXUIElementCopyAttributeValue(unsafeBitCast(application, to: AXUIElement.self), kAXFocusedWindowAttribute as CFString, &window) == .success, let window else { throw NativeFailure.message("No focused window") }
        guard let frame = windowFrame(unsafeBitCast(window, to: AXUIElement.self)), filter.contentRect.contains(frame) else {
            throw NativeFailure.message("Move the focused application window fully onto the shared display before using computer tools")
        }
    }

    private func windowFrame(_ window: AXUIElement) -> CGRect? {
        var position: CFTypeRef?
        var size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(window, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(window, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size, CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { return nil }
        var point = CGPoint.zero
        var dimensions = CGSize.zero
        guard AXValueGetValue(unsafeBitCast(position, to: AXValue.self), .cgPoint, &point), AXValueGetValue(unsafeBitCast(size, to: AXValue.self), .cgSize, &dimensions) else { return nil }
        return CGRect(origin: point, size: dimensions)
    }

    private func rect(_ rect: CGRect) -> [String: Double] { ["x":rect.origin.x,"y":rect.origin.y,"width":rect.width,"height":rect.height] }

    private func inspect(_ element: AXUIElement, depth: Int, remaining: inout Int) -> [String: Any] {
        remaining -= 1
        var result: [String: Any] = [:]
        for (attribute, name) in [(kAXRoleAttribute,"role"),(kAXSubroleAttribute,"subrole"),(kAXTitleAttribute,"title"),(kAXDescriptionAttribute,"description"),(kAXValueAttribute,"value")] {
            var value: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success, let value {
                if let string = value as? String { result[name] = String(string.prefix(1000)) }
            }
        }
        if result["role"] as? String == "AXSecureTextField" || result["subrole"] as? String == "AXSecureTextField" { result.removeValue(forKey: "value") }
        var children: CFTypeRef?
        if depth < 4 && remaining > 0 && AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children) == .success, let elements = children as? [AXUIElement] {
            var values: [[String: Any]] = []
            for child in elements { if remaining <= 0 { break }; values.append(inspect(child, depth: depth + 1, remaining: &remaining)) }
            result["children"] = values
        }
        return result
    }
}

@available(macOS 14.0, *)
final class DiscardedFrames: NSObject, SCStreamOutput {
    // Keeping a low-frequency stream active gives users the system sharing
    // indicator and stop controls. Frames enter model context only on capture.
    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of outputType: SCStreamOutputType) {}
}

@main
struct SolarNativeMain {
    @MainActor
    static func main() {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        let service = NativeService()
        Task.detached {
            while let line = readLine() {
                var id: Any = NSNull()
                do {
                    guard let data = line.data(using: .utf8), let request = try JSONSerialization.jsonObject(with: data) as? [String: Any], let action = request["action"] as? String else { throw NativeFailure.message("Invalid request") }
                    id = request["id"] ?? NSNull()
                    let result = try await service.handle(action, request["arguments"] as? [String: Any] ?? [:])
                    emit(["id":id,"result":result])
                } catch { emit(["id":id,"error":error.localizedDescription]) }
            }
            await MainActor.run { NSApplication.shared.terminate(nil) }
        }
        application.run()
    }

    private static func emit(_ value: [String: Any]) {
        if let data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), let text = String(data: data, encoding: .utf8) { print(text); fflush(stdout) }
    }
}
