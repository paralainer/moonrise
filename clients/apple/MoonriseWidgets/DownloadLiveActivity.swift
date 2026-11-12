import ActivityKit
import SwiftUI
import WidgetKit

private let accent = Color.white

struct DownloadLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: DownloadActivityAttributes.self) { context in
            LockScreenView(state: context.state)
                .padding(16)
                .activityBackgroundTint(Color.black.opacity(0.85))
                .activitySystemActionForegroundColor(accent)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: icon(context.state))
                        .font(.title2)
                        .foregroundStyle(accent)
                        .padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text(percent(context.state))
                        .font(.title3.monospacedDigit().weight(.semibold))
                        .foregroundStyle(.white)
                        .padding(.trailing, 4)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(context.state.title)
                        .font(.headline)
                        .lineLimit(1)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 6) {
                        ProgressView(value: context.state.progress)
                            .tint(accent)
                        HStack {
                            Text(context.state.detail)
                            Spacer()
                            Text(subtitle(context.state))
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 4)
                }
            } compactLeading: {
                Image(systemName: icon(context.state))
                    .foregroundStyle(accent)
            } compactTrailing: {
                Text(percent(context.state))
                    .font(.caption.monospacedDigit().weight(.semibold))
                    .foregroundStyle(accent)
            } minimal: {
                ProgressView(value: context.state.progress)
                    .progressViewStyle(.circular)
                    .tint(accent)
            }
            .keylineTint(accent)
        }
    }
}

private struct LockScreenView: View {
    let state: DownloadActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: icon(state))
                    .font(.title2)
                    .foregroundStyle(accent)
                VStack(alignment: .leading, spacing: 2) {
                    Text(state.title)
                        .font(.headline)
                        .foregroundStyle(.white)
                        .lineLimit(1)
                    Text(state.detail)
                        .font(.caption)
                        .foregroundStyle(.white.opacity(0.7))
                        .lineLimit(1)
                }
                Spacer()
                Text(percent(state))
                    .font(.title3.monospacedDigit().weight(.semibold))
                    .foregroundStyle(.white)
            }
            ProgressView(value: state.progress)
                .tint(accent)
            Text(subtitle(state))
                .font(.caption2)
                .foregroundStyle(.white.opacity(0.6))
        }
    }
}

private func icon(_ s: DownloadActivityAttributes.ContentState) -> String {
    if s.isFinished { return "checkmark.circle.fill" }
    if s.isPaused { return "pause.circle.fill" }
    return "arrow.down.circle.fill"
}

private func percent(_ s: DownloadActivityAttributes.ContentState) -> String {
    "\(Int((s.progress * 100).rounded()))%"
}

private func subtitle(_ s: DownloadActivityAttributes.ContentState) -> String {
    if s.isFinished { return "Download complete" }
    if s.isPaused { return "Paused" }
    var parts: [String] = []
    if s.bytesPerSecond > 0 {
        parts.append(ByteCountFormatter.string(fromByteCount: Int64(s.bytesPerSecond), countStyle: .file) + "/s")
    }
    if s.queued > 0 { parts.append("\(s.queued) more queued") }
    return parts.isEmpty ? "Downloading" : parts.joined(separator: " · ")
}
