import Foundation

/// Owns one OS thread for Bevy's thread-local main executor and all room FFI calls.
/// A serial DispatchQueue can switch OS threads between blocks.
final class LivingRoomThread {
    private final class State {
        let condition = NSCondition()
        var jobs: [() -> Void] = []
        var firstJob = 0
        var stopping = false
    }

    private let state = State()
    private let worker: Thread

    init() {
        let state = self.state
        worker = Thread {
            while true {
                state.condition.lock()
                while state.jobs.isEmpty && !state.stopping {
                    state.condition.wait()
                }
                guard !state.jobs.isEmpty else {
                    state.condition.unlock()
                    return
                }
                let job = state.jobs[state.firstJob]
                state.firstJob += 1
                if state.firstJob == state.jobs.count {
                    state.jobs.removeAll(keepingCapacity: true)
                    state.firstJob = 0
                } else if state.firstJob >= 64 {
                    state.jobs.removeFirst(state.firstJob)
                    state.firstJob = 0
                }
                state.condition.unlock()
                autoreleasepool {
                    job()
                }
            }
        }
        worker.name = "dev.specchum.living-room"
        worker.qualityOfService = .userInteractive
        worker.start()
    }

    func async(execute job: @escaping () -> Void) {
        state.condition.lock()
        state.jobs.append(job)
        state.condition.signal()
        state.condition.unlock()
    }

    func sync(execute job: @escaping () -> Void) {
        if Thread.current === worker {
            job()
            return
        }
        let finished = DispatchSemaphore(value: 0)
        async {
            defer { finished.signal() }
            job()
        }
        finished.wait()
    }

    deinit {
        state.condition.lock()
        state.stopping = true
        state.condition.signal()
        state.condition.unlock()
    }
}
