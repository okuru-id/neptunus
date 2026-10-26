// Films on iOS play here rather than in the WebView's <video>: VLCKit reads MKV, HEVC, AC3/DTS and the rest,
// which AVPlayer won't. The CI build copies this file into the generated Xcode project and links
// MobileVLCKit (scripts/ios-vlckit.sh). Rust starts it (`video::ios_play_native`) with
// `{url, title, headers, position}` (ms), hands it the subtitles as files once they are fetched
// (`addSubtitle`), and hears where it got to when it closes (`neptunus_video_closed`).

import MobileVLCKit
import UIKit

@_silgen_name("neptunus_video_closed")
func neptunusVideoClosed(_ json: UnsafePointer<CChar>)

@objc(NeptunusPlayer) public class NeptunusPlayer: NSObject {
  fileprivate static var current: PlayerController?

  @objc public static func play(_ json: NSString) {
    guard let data = (json as String).data(using: .utf8),
      let o = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let top = topController()
    else { return }
    current?.close()
    let vc = PlayerController(o)
    current = vc
    top.present(vc, animated: true)
  }

  /// A subtitle file for the film playing; `select` shows it.
  @objc public static func addSubtitle(_ path: NSString, select: Bool) {
    current?.addSubtitle(URL(fileURLWithPath: path as String), select: select)
  }

  private static func topController() -> UIViewController? {
    let window = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
      .flatMap { $0.windows }.first { $0.isKeyWindow }
    var top = window?.rootViewController
    while let next = top?.presentedViewController { top = next }
    return top
  }
}

fileprivate final class PlayerController: UIViewController, VLCMediaPlayerDelegate {
  private let player = VLCMediaPlayer()
  private let videoView = UIView()
  private let controls = UIView()
  private let titleLabel = UILabel()
  private let playButton = UIButton(type: .system)
  private let slider = UISlider()
  private let timeLabel = UILabel()
  private let lengthLabel = UILabel()
  private let spinner = UIActivityIndicatorView(style: .large)
  private var timer: Timer?
  private var hideAt = Date.distantFuture
  private var seeking = false
  private var reported = false
  private var failed: String?
  private let options: [String: Any]

  init(_ o: [String: Any]) {
    options = o
    super.init(nibName: nil, bundle: nil)
    modalPresentationStyle = .fullScreen
  }

  required init?(coder: NSCoder) { fatalError() }

  override var supportedInterfaceOrientations: UIInterfaceOrientationMask { .landscape }
  override var prefersStatusBarHidden: Bool { true }
  override var prefersHomeIndicatorAutoHidden: Bool { true }

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .black
    videoView.frame = view.bounds
    videoView.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    view.addSubview(videoView)
    buildControls()
    view.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(toggleControls)))

    guard let url = URL(string: options["url"] as? String ?? "") else { return }
    let media = VLCMedia(url: url)
    // VLC takes only these two of the headers a stream's host asks for
    let headers = options["headers"] as? [String: String] ?? [:]
    for (k, v) in headers {
      switch k.lowercased() {
      case "user-agent": media.addOption(":http-user-agent=\(v)")
      case "referer": media.addOption(":http-referrer=\(v)")
      default: break
      }
    }
    let start = (options["position"] as? Double ?? 0) / 1000
    if start > 0 { media.addOption(":start-time=\(start)") }
    player.media = media
    player.drawable = videoView
    player.delegate = self
    player.play()
    timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in self?.tick() }
  }

  override func viewDidAppear(_ animated: Bool) {
    super.viewDidAppear(animated)
    // turn sideways: the rest of the app is portrait
    if #available(iOS 16.0, *) {
      setNeedsUpdateOfSupportedInterfaceOrientations()
      view.window?.windowScene?.requestGeometryUpdate(.iOS(interfaceOrientations: .landscape))
    } else {
      UIDevice.current.setValue(UIInterfaceOrientation.landscapeRight.rawValue, forKey: "orientation")
    }
  }

  func addSubtitle(_ url: URL, select: Bool) {
    player.addPlaybackSlave(url, type: .subtitle, enforce: select)
  }

  // --- controls ---

  private func buildControls() {
    controls.frame = view.bounds
    controls.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    controls.backgroundColor = UIColor.black.withAlphaComponent(0.35)
    view.addSubview(controls)

    let close = iconButton("xmark", #selector(closeTapped))
    titleLabel.text = options["title"] as? String
    titleLabel.textColor = .white
    titleLabel.font = .boldSystemFont(ofSize: 17)
    let top = UIStackView(arrangedSubviews: [close, titleLabel])
    top.spacing = 12

    playButton.setImage(UIImage(systemName: "pause.fill"), for: .normal)
    playButton.tintColor = .white
    playButton.addTarget(self, action: #selector(playTapped), for: .touchUpInside)
    for label in [timeLabel, lengthLabel] {
      label.textColor = .white
      label.font = .monospacedDigitSystemFont(ofSize: 14, weight: .medium)
      label.text = "0:00"
    }
    slider.minimumTrackTintColor = .white
    slider.addTarget(self, action: #selector(seekStarted), for: .touchDown)
    slider.addTarget(self, action: #selector(seekEnded), for: [.touchUpInside, .touchUpOutside, .touchCancel])
    let bottom = UIStackView(arrangedSubviews: [
      playButton, timeLabel, slider, lengthLabel,
      iconButton("captions.bubble", #selector(subtitlesTapped)),
      iconButton("speaker.wave.2", #selector(audioTapped)),
    ])
    bottom.spacing = 12
    bottom.alignment = .center

    for bar in [top, bottom] {
      bar.translatesAutoresizingMaskIntoConstraints = false
      controls.addSubview(bar)
    }
    spinner.color = .white
    spinner.translatesAutoresizingMaskIntoConstraints = false
    view.addSubview(spinner)
    let g = controls.safeAreaLayoutGuide
    NSLayoutConstraint.activate([
      top.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: 16),
      top.trailingAnchor.constraint(lessThanOrEqualTo: g.trailingAnchor, constant: -16),
      top.topAnchor.constraint(equalTo: g.topAnchor, constant: 12),
      bottom.leadingAnchor.constraint(equalTo: g.leadingAnchor, constant: 16),
      bottom.trailingAnchor.constraint(equalTo: g.trailingAnchor, constant: -16),
      bottom.bottomAnchor.constraint(equalTo: g.bottomAnchor, constant: -12),
      spinner.centerXAnchor.constraint(equalTo: view.centerXAnchor),
      spinner.centerYAnchor.constraint(equalTo: view.centerYAnchor),
    ])
    spinner.startAnimating()
    wake()
  }

  private func iconButton(_ name: String, _ action: Selector) -> UIButton {
    let b = UIButton(type: .system)
    b.setImage(UIImage(systemName: name), for: .normal)
    b.tintColor = .white
    b.addTarget(self, action: action, for: .touchUpInside)
    b.widthAnchor.constraint(equalToConstant: 40).isActive = true
    b.heightAnchor.constraint(equalToConstant: 40).isActive = true
    return b
  }

  /// Controls up, hidden again after a while of playing.
  private func wake() {
    controls.isHidden = false
    hideAt = Date().addingTimeInterval(4)
  }

  @objc private func toggleControls() {
    if controls.isHidden { wake() } else { controls.isHidden = true }
  }

  @objc private func playTapped() {
    if player.isPlaying { player.pause() } else { player.play() }
    wake()
  }

  @objc private func seekStarted() { seeking = true }

  @objc private func seekEnded() {
    player.position = slider.value
    seeking = false
    wake()
  }

  @objc private func closeTapped() { close() }

  @objc private func subtitlesTapped(_ sender: UIButton) {
    let names = player.videoSubTitlesNames as? [String] ?? []
    let ids = player.videoSubTitlesIndexes as? [NSNumber] ?? []
    pick("Subtitle", zip(names, ids).map { ($0.0, $0.1.int32Value) }, player.currentVideoSubTitleIndex, sender) {
      self.player.currentVideoSubTitleIndex = $0
    }
  }

  @objc private func audioTapped(_ sender: UIButton) {
    let names = player.audioTrackNames as? [String] ?? []
    let ids = player.audioTrackIndexes as? [NSNumber] ?? []
    pick("Audio", zip(names, ids).map { ($0.0, $0.1.int32Value) }, player.currentAudioTrackIndex, sender) {
      self.player.currentAudioTrackIndex = $0
    }
  }

  private func pick(_ title: String, _ items: [(String, Int32)], _ on: Int32, _ from: UIView, _ set: @escaping (Int32) -> Void) {
    let sheet = UIAlertController(title: title, message: nil, preferredStyle: .actionSheet)
    for (name, id) in items {
      sheet.addAction(UIAlertAction(title: (id == on ? "✓ " : "") + name, style: .default) { _ in set(id) })
    }
    sheet.addAction(UIAlertAction(title: "Tutup", style: .cancel))
    sheet.popoverPresentationController?.sourceView = from
    sheet.popoverPresentationController?.sourceRect = from.bounds
    present(sheet, animated: true)
    hideAt = .distantFuture
  }

  private func tick() {
    let time = Double(player.time.intValue) / 1000
    let length = Double(player.media?.length.intValue ?? 0) / 1000
    timeLabel.text = clock(time)
    lengthLabel.text = clock(length)
    if !seeking { slider.value = player.position }
    playButton.setImage(UIImage(systemName: player.isPlaying ? "pause.fill" : "play.fill"), for: .normal)
    if player.isPlaying, !seeking, Date() > hideAt { controls.isHidden = true }
  }

  private func clock(_ s: Double) -> String {
    let t = Int(max(0, s))
    return t >= 3600 ? String(format: "%d:%02d:%02d", t / 3600, t / 60 % 60, t % 60) : String(format: "%d:%02d", t / 60, t % 60)
  }

  func mediaPlayerStateChanged(_ aNotification: Notification) {
    switch player.state {
    case .buffering, .opening: if !player.isPlaying { spinner.startAnimating() }
    case .error:
      failed = "VLC tidak bisa membuka film ini"
      close()
    case .ended: close()
    default: spinner.stopAnimating()
    }
  }

  func mediaPlayerTimeChanged(_ aNotification: Notification) {
    spinner.stopAnimating()
  }

  // --- closing: once, saying where it got to ---

  func close() {
    guard !reported else { return }
    reported = true
    timer?.invalidate()
    let json: [String: Any] = [
      "pos": Double(player.time.intValue) / 1000,
      "duration": Double(player.media?.length.intValue ?? 0) / 1000,
      "eof": player.state == .ended,
      "error": failed ?? NSNull(),
    ]
    player.stop()
    if NeptunusPlayer.current === self { NeptunusPlayer.current = nil }
    dismiss(animated: true)
    if let data = try? JSONSerialization.data(withJSONObject: json), let s = String(data: data, encoding: .utf8) {
      s.withCString { neptunusVideoClosed($0) }
    }
  }
}
