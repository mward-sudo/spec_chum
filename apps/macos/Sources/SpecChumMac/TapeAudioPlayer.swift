import AVFoundation
import Foundation

/// Plays mono PCM frames from `host_api` (`sc_audio_*`) via AVAudioEngine.
final class TapeAudioPlayer {
    private let engine = AVAudioEngine()
    private let player = AVAudioPlayerNode()
    private var format: AVAudioFormat?
    private var started = false

    func ensureStarted(sampleRate: Double) {
        if started, format?.sampleRate == sampleRate {
            return
        }
        stop()
        guard let fmt = AVAudioFormat(standardFormatWithSampleRate: sampleRate, channels: 1) else {
            return
        }
        format = fmt
        engine.attach(player)
        engine.connect(player, to: engine.mainMixerNode, format: fmt)
        engine.mainMixerNode.outputVolume = 1.0
        do {
            try engine.start()
            player.play()
            started = true
        } catch {
            started = false
        }
    }

    func stop() {
        if started {
            player.stop()
            engine.stop()
        }
        if engine.attachedNodes.contains(player) {
            engine.disconnectNodeOutput(player)
            engine.detach(player)
        }
        started = false
        format = nil
    }

    /// Schedule one frame of mono f32 samples (copied immediately).
    func schedule(samples: UnsafePointer<Float>, count: Int) {
        guard started, let format, count > 0 else { return }
        guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(count))
        else { return }
        buffer.frameLength = AVAudioFrameCount(count)
        if let dst = buffer.floatChannelData?[0] {
            dst.update(from: samples, count: count)
        }
        player.scheduleBuffer(buffer, completionHandler: nil)
    }
}
