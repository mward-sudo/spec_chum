import AppKit
import Foundation
import GameController
import IOSurface
import UniformTypeIdentifiers
import CSpecChumHost

extension HostBridge {
    func enableDefaultTrace() {
        sc_debug_set_categories(2 | 4 | 8 | 16)
        status = "Trace enabled (default categories), events=\(sc_debug_event_count())"
    }

    func clearTrace() {
        sc_debug_clear()
        status = "Trace ring cleared"
    }

    func dumpTraceToDesktop() {
        let dir = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let url = dir.appendingPathComponent("spec_chum_trace.txt")
        dumpTrace(to: url)
    }

    func dumpTracePanel() {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = "spec_chum_trace.txt"
        panel.allowedContentTypes = [.plainText]
        if panel.runModal() == .OK, let url = panel.url {
            dumpTrace(to: url)
        }
    }

    func dumpTrace(to url: URL) {
        let ok = url.path.withCString { sc_debug_dump_to_file($0) }
        if ok != 0 {
            status = HostBridge.takeLastError() ?? "Trace dump failed"
        } else {
            status = "Trace dump → \(url.path) (\(sc_debug_event_count()) events in ring)"
        }
    }

    func peek(addr: UInt32) -> UInt8? {
        guard let handle else { return nil }
        var out: UInt8 = 0
        guard sc_peek(handle, addr, &out) == 0 else { return nil }
        return out
    }

    @discardableResult
    func poke(addr: UInt32, value: UInt8) -> Bool {
        guard let handle else { return false }
        return sc_poke(handle, addr, value) == 0
    }

    func inspectJson() -> String? {
        guard let handle, let cstr = sc_inspect_json(handle) else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    func regs() -> (pc: UInt16, sp: UInt16, af: UInt16, bc: UInt16, de: UInt16, hl: UInt16, ix: UInt16, iy: UInt16)? {
        guard let handle else { return nil }
        var pc: UInt16 = 0
        var sp: UInt16 = 0
        var af: UInt16 = 0
        var bc: UInt16 = 0
        var de: UInt16 = 0
        var hl: UInt16 = 0
        var ix: UInt16 = 0
        var iy: UInt16 = 0
        guard sc_regs(handle, &pc, &sp, &af, &bc, &de, &hl, &ix, &iy) == 0 else {
            return nil
        }
        return (pc, sp, af, bc, de, hl, ix, iy)
    }

    func step() {
        guard let handle else { return }
        if sc_step(handle) != 0 {
            status = HostBridge.takeLastError() ?? "Step failed"
            return
        }
        refreshInspector()
    }

    func setPaused(_ value: Bool) {
        paused = value
        status = value ? "Paused" : "Running"
        refreshInspector()
    }

    @discardableResult
    func addBreakpoint(pc: UInt32) -> Bool {
        guard let handle else { return false }
        if sc_add_breakpoint(handle, pc) != 0 {
            status = HostBridge.takeLastError() ?? "Breakpoint failed"
            return false
        }
        status = String(format: "Breakpoint at PC %04X", pc)
        return true
    }

    func addBreakpointAtPc() {
        guard let r = regs() else {
            status = HostBridge.takeLastError() ?? "No machine"
            return
        }
        _ = addBreakpoint(pc: UInt32(r.pc))
        refreshInspector()
    }

    @discardableResult
    func runUntilBreak(maxInsns: UInt32 = 1_000_000) -> Int32 {
        guard let handle else { return -1 }
        let reason = sc_run_until_break(handle, maxInsns)
        if reason < 0 {
            status = HostBridge.takeLastError() ?? "run-until-break failed"
        } else {
            status = "Break reason \(reason)"
            if reason == 1 || reason == 2 || reason == 3 || reason == 4 {
                suppressPausedPush = true
                paused = true
                suppressPausedPush = false
            }
        }
        refreshInspector()
        return reason
    }

    func dumpTraceJson() -> String? {
        guard let cstr = sc_debug_dump_json() else { return nil }
        let s = String(cString: cstr)
        sc_string_free(cstr)
        return s
    }

    func dumpTraceJsonToDesktop() {
        let dir = FileManager.default.urls(for: .desktopDirectory, in: .userDomainMask).first
            ?? URL(fileURLWithPath: NSTemporaryDirectory())
        let url = dir.appendingPathComponent("spec_chum_trace.json")
        guard let json = dumpTraceJson() else {
            status = "JSON dump failed"
            return
        }
        do {
            try json.write(to: url, atomically: true, encoding: .utf8)
            status = "Trace JSON → \(url.path) (\(sc_debug_event_count()) events in ring)"
        } catch {
            status = "Trace JSON write failed: \(error.localizedDescription)"
        }
    }

    func refreshInspector() {
        if let r = regs() {
            debugPc = r.pc
            debugSp = r.sp
            debugAf = r.af
        }
        if let json = inspectJson() {
            inspectJsonPreview = String(json.prefix(2000))
        } else {
            inspectJsonPreview = "(no machine)"
        }
    }

}
