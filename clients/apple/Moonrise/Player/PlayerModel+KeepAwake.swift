import Foundation

extension PlayerModel {
    func updateKeepAwake() {
        keepAwake.set(KeepAwake.shouldHold(isPlaying: isPlaying, rate: rate, isFinished: isFinished,
                                           hasError: errorMessage != nil, closed: closed),
                      reason: KeepAwake.reason(title: title))
    }
}
