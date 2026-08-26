import AudioToolbox
import Foundation

/// Plays mono PCM frames from `host_api` (`sc_audio_*`).
///
/// Uses **AudioQueue** (not AVAudioEngine/PlayerNode). On macOS 27 / Tahoe betas,
/// AVAudioEngine reported `isRunning` and accepted buffers / source-node attaches,
/// but the render thread stopped after a handful of callbacks (`completions=0`,
/// `renders` stuck) → permanent silence despite non-zero host PCM.
///
/// Gain / mute affect **host output only** — not EAR bit fidelity or flash-load.
/// Never touch macOS system output volume from here or from automation.
final class TapeAudioPlayer {
    private var queue: AudioQueueRef?
    private var started = false
    private var hostSampleRate: Double = 44_100

    /// Fixed-capacity ring — no realloc / `removeFirst` in the AudioQueue callback.
    private let ringLock = NSLock()
    private var ring: ContiguousArray<Float>
    private var ringHead = 0
    private var ringCount = 0
    private static let maxRing = 44_100 * 2

    private static let debug =
        ProcessInfo.processInfo.environment["SPEC_CHUM_AUDIO_DEBUG"] == "1"
    private static let captureEnabled =
        ProcessInfo.processInfo.environment["SPEC_CHUM_AUDIO_CAPTURE"] == "1"
    private var debugSchedules: UInt64 = 0
    /// Written only under `ringLock` (callback + schedule debug path).
    private var callbackCount: UInt64 = 0
    /// Last AudioQueueEnqueueBuffer error from the realtime callback (reported on schedule).
    private var pendingEnqueueError: OSStatus = noErr
    private var capture: AudioCaptureFile?

    private var gain: Float = 1.0
    private var mutedFlag = false

    init() {
        ring = ContiguousArray(repeating: 0, count: Self.maxRing)
    }

    var volume: Float = 1.0 {
        didSet { applyGain() }
    }

    var muted: Bool = false {
        didSet { applyGain() }
    }

    @discardableResult
    func ensureStarted(sampleRate: Double, force: Bool = false) -> Bool {
        hostSampleRate = sampleRate > 0 ? sampleRate : 44_100
        if started, !force, queue != nil {
            applyGain()
            return true
        }
        stop()
        return startQueue()
    }

    private func startQueue() -> Bool {
        var asbd = AudioStreamBasicDescription(
            mSampleRate: hostSampleRate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat
                | kAudioFormatFlagIsPacked
                | kLinearPCMFormatFlagIsNonInterleaved,
            mBytesPerPacket: UInt32(MemoryLayout<Float>.size),
            mFramesPerPacket: 1,
            mBytesPerFrame: UInt32(MemoryLayout<Float>.size),
            mChannelsPerFrame: 1,
            mBitsPerChannel: 32,
            mReserved: 0
        )

        let client = Unmanaged.passUnretained(self).toOpaque()
        var q: AudioQueueRef?
        let status = AudioQueueNewOutput(
            &asbd,
            tapeAudioQueueOutputCallback,
            client,
            nil,
            nil,
            0,
            &q
        )
        guard status == noErr, let q else {
            errorLog("AudioQueueNewOutput failed: \(status)")
            return false
        }
        queue = q

        // Prime several buffers so the queue stays ahead of the 50 Hz host tick.
        let framesPerBuffer = UInt32(hostSampleRate / 50) // ~20 ms
        for _ in 0..<4 {
            var buf: AudioQueueBufferRef?
            let alloc = AudioQueueAllocateBuffer(
                q,
                framesPerBuffer * UInt32(MemoryLayout<Float>.size),
                &buf
            )
            guard alloc == noErr, let buf else {
                errorLog("AudioQueueAllocateBuffer failed: \(alloc)")
                stop()
                return false
            }
            buf.pointee.mAudioDataByteSize = 0
            Self.fill(buffer: buf, player: self)
            let enq = AudioQueueEnqueueBuffer(q, buf, 0, nil)
            if enq != noErr {
                errorLog("AudioQueueEnqueueBuffer failed: \(enq)")
                stop()
                return false
            }
        }

        applyGain()
        let start = AudioQueueStart(q, nil)
        guard start == noErr else {
            errorLog("AudioQueueStart failed: \(start)")
            stop()
            return false
        }
        started = true

        if Self.captureEnabled {
            capture = AudioCaptureFile(
                path: "/tmp/spec-chum-capture.wav",
                sampleRate: hostSampleRate
            )
        }
        debugLog(
            "ensureStarted: AudioQueue @ \(hostSampleRate) Hz gain=\(gain) muted=\(mutedFlag) capture=\(Self.captureEnabled)"
        )
        return true
    }

    func stop() {
        if let capture {
            let stats = capture.finalize()
            ringLock.lock()
            let callbacks = callbackCount
            ringLock.unlock()
            debugLog(
                "capture finalize: path=\(stats.path) frames=\(stats.frames) peak=\(stats.peak) rms=\(stats.rms) callbacks=\(callbacks)"
            )
            self.capture = nil
        }
        if let q = queue {
            AudioQueueStop(q, true)
            AudioQueueDispose(q, true)
        }
        queue = nil
        started = false
        ringLock.lock()
        callbackCount = 0
        pendingEnqueueError = noErr
        ringHead = 0
        ringCount = 0
        ringLock.unlock()
    }

    /// Enqueue one frame of mono f32 samples (host rate). Engine must already be started.
    func schedule(samples: UnsafePointer<Float>, count: Int) {
        guard count > 0, started else { return }
        if Self.captureEnabled {
            capture?.append(Array(UnsafeBufferPointer(start: samples, count: count)))
        }

        ringLock.lock()
        let enqueueErr = pendingEnqueueError
        pendingEnqueueError = noErr
        let cap = Self.maxRing
        var idx = ringHead
        var len = ringCount
        var i = 0
        while i < count {
            if len == cap {
                idx = (idx + 1) % cap
                len -= 1
            }
            let write = (idx + len) % cap
            ring[write] = samples[i]
            len += 1
            i += 1
        }
        ringHead = idx
        ringCount = len
        let qLen = len
        let callbacks = callbackCount
        ringLock.unlock()

        if enqueueErr != noErr {
            errorLog("AudioQueueEnqueueBuffer (callback) failed: \(enqueueErr)")
        }

        if Self.debug {
            debugSchedules &+= 1
            if debugSchedules == 1 || debugSchedules % 250 == 0 {
                var peak: Float = 0
                for j in 0..<count {
                    peak = max(peak, abs(samples[j]))
                }
                debugLog(
                    "schedule: +\(count) peak=\(peak) ring=\(qLen) callbacks=\(callbacks)"
                )
            }
        }
    }

    private func applyGain() {
        gain = muted ? 0 : max(0, min(1, volume))
        mutedFlag = muted
        if let q = queue {
            AudioQueueSetParameter(q, kAudioQueueParam_Volume, gain)
        }
    }

    /// Realtime-safe fill: lock + O(n) ring copy only — no alloc, log, or file I/O.
    fileprivate static func fill(buffer: AudioQueueBufferRef, player: TapeAudioPlayer) {
        let capacity = Int(buffer.pointee.mAudioDataBytesCapacity) / MemoryLayout<Float>.size
        guard capacity > 0 else {
            buffer.pointee.mAudioDataByteSize = 0
            return
        }
        let out = buffer.pointee.mAudioData.assumingMemoryBound(to: Float.self)

        player.ringLock.lock()
        let n = min(capacity, player.ringCount)
        if n > 0 {
            let head = player.ringHead
            let cap = Self.maxRing
            for i in 0..<n {
                out[i] = player.ring[(head + i) % cap]
            }
            player.ringHead = (head + n) % cap
            player.ringCount -= n
        }
        player.callbackCount &+= 1
        player.ringLock.unlock()

        for i in n..<capacity {
            out[i] = 0
        }
        buffer.pointee.mAudioDataByteSize = UInt32(capacity * MemoryLayout<Float>.size)
    }

    fileprivate func noteEnqueueError(_ status: OSStatus) {
        ringLock.lock()
        pendingEnqueueError = status
        ringLock.unlock()
    }

    private func debugLog(_ message: String) {
        guard Self.debug || Self.captureEnabled else { return }
        AudioLog.write(message)
    }

    fileprivate func errorLog(_ message: String) {
        AudioLog.write("ERROR \(message)", force: true)
    }
}

private func tapeAudioQueueOutputCallback(
    userData: UnsafeMutableRawPointer?,
    aq: AudioQueueRef,
    buffer: AudioQueueBufferRef
) {
    guard let userData else { return }
    let player = Unmanaged<TapeAudioPlayer>.fromOpaque(userData).takeUnretainedValue()
    TapeAudioPlayer.fill(buffer: buffer, player: player)
    let enq = AudioQueueEnqueueBuffer(aq, buffer, 0, nil)
    if enq != noErr {
        // Defer logging to the main-thread schedule path (no I/O here).
        player.noteEnqueueError(enq)
    }
}

// MARK: - WAV capture (SPEC_CHUM_AUDIO_CAPTURE=1)

private final class AudioCaptureFile {
    struct Stats {
        let path: String
        let frames: Int
        let peak: Float
        let rms: Float
    }

    private let path: String
    private let sampleRate: Double
    private let lock = NSLock()
    private var pcm = Data()
    private var frames = 0
    private var peak: Float = 0
    private var sumSq: Double = 0
    private var finalized = false
    private var framesSinceFlush = 0
    private static let flushEveryFrames = 22_050

    init(path: String, sampleRate: Double) {
        self.path = path
        self.sampleRate = sampleRate
        try? FileManager.default.removeItem(atPath: path)
        AudioLog.write("capture open: \(path) @ \(sampleRate) Hz", force: true)
    }

    func append(_ samples: [Float]) {
        guard !samples.isEmpty else { return }
        var shouldFlush = false
        lock.lock()
        if !finalized {
            for s in samples {
                var v = s
                withUnsafeBytes(of: &v) { pcm.append(contentsOf: $0) }
                peak = max(peak, abs(s))
                sumSq += Double(s) * Double(s)
            }
            frames += samples.count
            framesSinceFlush += samples.count
            if framesSinceFlush >= Self.flushEveryFrames {
                framesSinceFlush = 0
                shouldFlush = true
            }
        }
        let snapshot = shouldFlush ? (pcm, frames, peak, sumSq) : nil
        lock.unlock()
        if let (data, n, p, sq) = snapshot {
            writeWav(path: path, sampleRate: sampleRate, float32LE: data)
            let rms = n > 0 ? Float(sqrt(sq / Double(n))) : 0
            AudioLog.write(
                "capture flush: \(path) frames=\(n) peak=\(p) rms=\(rms)",
                force: true
            )
        }
    }

    @discardableResult
    func finalize() -> Stats {
        lock.lock()
        if finalized {
            let n = max(frames, 1)
            let stats = Stats(
                path: path,
                frames: frames,
                peak: peak,
                rms: Float(sqrt(sumSq / Double(n)))
            )
            lock.unlock()
            return stats
        }
        finalized = true
        let data = pcm
        let n = frames
        let p = peak
        let rms = n > 0 ? Float(sqrt(sumSq / Double(n))) : 0
        lock.unlock()

        writeWav(path: path, sampleRate: sampleRate, float32LE: data)
        let stats = Stats(path: path, frames: n, peak: p, rms: rms)
        AudioLog.write(
            "capture wrote: \(path) frames=\(n) peak=\(p) rms=\(rms) bytes=\(data.count)",
            force: true
        )
        return stats
    }

    private func writeWav(path: String, sampleRate: Double, float32LE: Data) {
        let dataSize = UInt32(float32LE.count)
        var header = Data()
        func appendASCII(_ s: String) { header.append(contentsOf: s.utf8) }
        func appendU32(_ v: UInt32) {
            var le = v.littleEndian
            withUnsafeBytes(of: &le) { header.append(contentsOf: $0) }
        }
        func appendU16(_ v: UInt16) {
            var le = v.littleEndian
            withUnsafeBytes(of: &le) { header.append(contentsOf: $0) }
        }
        appendASCII("RIFF")
        appendU32(36 + dataSize)
        appendASCII("WAVE")
        appendASCII("fmt ")
        appendU32(16)
        appendU16(3)
        appendU16(1)
        appendU32(UInt32(sampleRate))
        appendU32(UInt32(sampleRate) * 4)
        appendU16(4)
        appendU16(32)
        appendASCII("data")
        appendU32(dataSize)
        var out = header
        out.append(float32LE)
        do {
            try out.write(to: URL(fileURLWithPath: path), options: .atomic)
        } catch {
            AudioLog.write("ERROR capture write failed: \(error.localizedDescription)", force: true)
        }
    }
}

private enum AudioLog {
    static func write(_ message: String, force: Bool = false) {
        let debug = ProcessInfo.processInfo.environment["SPEC_CHUM_AUDIO_DEBUG"] == "1"
            || ProcessInfo.processInfo.environment["SPEC_CHUM_AUDIO_CAPTURE"] == "1"
        guard debug || force else { return }
        let line = "spec-chum-audio: \(message)"
        NSLog("%@", line)
        fputs("\(line)\n", stderr)
        if debug {
            let url = URL(fileURLWithPath: "/tmp/spec-chum-audio.log")
            let payload = Data("\(line)\n".utf8)
            if let h = try? FileHandle(forWritingTo: url) {
                h.seekToEndOfFile()
                h.write(payload)
                try? h.close()
            } else {
                try? payload.write(to: url)
            }
        }
    }
}
